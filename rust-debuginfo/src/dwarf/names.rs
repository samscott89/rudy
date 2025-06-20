//! Various parsing of names used in DWARF files

use std::fmt;

use anyhow::Context;
use unsynn::ToTokens;

mod parser;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleName {
    pub segments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeName {
    module: ModuleName,
    name: String,
}

impl TypeName {
    pub fn parse(module_path: &[String], name: &str) -> anyhow::Result<Self> {
        fn known_bad_case(path: &str) -> bool {
            path.contains("{closure_env#")
        }
        let full_path = parser::parse_type(name).map_err(|e| {
            if !known_bad_case(name) {
                tracing::error!("Failed to parse type name `{name}`: {e}");
            }
            anyhow::anyhow!("Failed to parse type name `{name}`")
        })?;

        // Kinda of a silly way to do it -- we'll probably want to use
        // the parsed type directly
        let type_name = full_path.to_string();
        Ok(TypeName {
            module: ModuleName {
                segments: module_path.to_vec(),
            },
            name: type_name,
        })
    }
}

impl fmt::Display for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.module.segments.is_empty() {
            write!(f, "{}::", self.module.segments.join("::"))?;
        }
        write!(f, "{}", self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol {
    name_bytes: Vec<u8>,
}

impl Symbol {
    pub fn new(name_bytes: Vec<u8>) -> Self {
        Self { name_bytes }
    }

    pub fn demangle(&self) -> anyhow::Result<SymbolName> {
        demangle_symbol(self.clone())
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SymbolName {
    pub lookup_name: String,
    pub hash: String,
    module_path: Vec<String>,
    full_path: String,
}

impl SymbolName {
    pub fn parse(path: &str) -> anyhow::Result<Self> {
        fn known_bad_case(path: &str) -> bool {
            path.contains('@')
                || path.contains("{{")
                || path.contains("__rustc[")
                || path.starts_with('$')
                || path.contains("DW.ref.rust_eh_personality")
                || path.ends_with(".o")
                || path.ends_with(".c")
                || path.ends_with("cgu.0")
                || path.starts_with("compiler_builtins.")
                || path.starts_with(|c: char| c.is_ascii_digit())
                || path.ends_with(".0")
                || path.ends_with(".0$tlv$init")
        }

        let full_path = parser::parse_path(path).map_err(|e| {
            // known bad cases we don't care about
            if !known_bad_case(path) {
                tracing::error!("Failed to parse symbol path `{path}`: {e}");
            }
            anyhow::anyhow!("Failed to parse symbol path `{path}`")
        })?;
        let mut segments = full_path.segments();
        let last = segments
            .pop()
            .context("No segments in demangled path")?
            .to_string();

        let hash = if last.starts_with('h') && last.chars().skip(1).all(|c| c.is_ascii_hexdigit()) {
            // This is a hash, we don't want it as the lookup name
            last
        } else {
            "".to_string()
        };

        let lookup_name = segments
            .pop()
            .context("No name in demangled path")?
            .to_string();

        Ok(SymbolName {
            full_path: path.to_string(),
            lookup_name,
            hash,
            module_path: segments,
        })
    }
}

impl fmt::Debug for SymbolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.full_path)
    }
}

impl fmt::Display for SymbolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            // print without the trailing hash
            self.full_path
                .trim_end_matches(&self.hash)
                .trim_end_matches("::")
        )
    }
}

impl SymbolName {
    pub fn matches_name_and_module(&self, name: &str, module: &[String]) -> bool {
        self.lookup_name == name && self.module_path.ends_with(module)
    }
}

impl PartialOrd for SymbolName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SymbolName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.lookup_name
            .cmp(&other.lookup_name)
            .then_with(|| self.module_path.cmp(&other.module_path))
            .then_with(|| self.full_path.cmp(&other.full_path))
    }
}

fn demangle_symbol(symbol: Symbol) -> anyhow::Result<SymbolName> {
    let name_str = std::str::from_utf8(&symbol.name_bytes)
        .context("Failed to convert symbol bytes to string")?;
    let name_str = if name_str.starts_with("__Z") {
        // Strip the extra leading `_` if it exists
        // this is a macos trait
        &name_str[1..]
    } else {
        name_str
    };
    let demangled = rustc_demangle::try_demangle(name_str.as_ref())
        .map_err(|_| anyhow::anyhow!("could not demangle symbol as Rust symbol"))?;
    SymbolName::parse(&demangled.to_string()).context("Failed to parse demangled symbol")
}

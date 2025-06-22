//! Various parsing of names used in DWARF files

use std::fmt;

use anyhow::Context;

mod parser;

use crate::typedef::TypeDef;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleName {
    pub segments: Vec<String>,
}

#[derive(Clone)]
pub struct TypeName {
    /// The module this type is defined in
    /// e.g. `alloc::string`
    pub module: ModuleName,
    /// The simple name of the type, e.g. `String`
    pub name: String,
    /// The full name of the type, including module path
    /// e.g. `alloc::string::String`
    pub full_name: String,
    pub typedef: TypeDef,
}

impl fmt::Debug for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl PartialEq for TypeName {
    fn eq(&self, other: &Self) -> bool {
        self.module == other.module && self.full_name == other.full_name
    }
}
impl Eq for TypeName {}
impl PartialOrd for TypeName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TypeName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.module
            .cmp(&other.module)
            .then_with(|| self.full_name.cmp(&other.full_name))
    }
}

impl std::hash::Hash for TypeName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.module.hash(state);
        self.full_name.hash(state);
    }
}

impl TypeName {
    pub fn parse(module_path: &[String], name: &str) -> anyhow::Result<Self> {
        fn known_bad_case(path: &str) -> bool {
            path.contains("{closure_env#") || path.contains("{impl#") || path.contains("{extern#")
        }

        // If we have a module path, prepend it to the name for parsing
        // This allows the parser to correctly identify std types like "String" as "alloc::string::String"
        let full_name = if module_path.is_empty() {
            name.to_string()
        } else {
            format!("{}::{}", module_path.join("::"), name)
        };

        tracing::debug!(
            "TypeName::parse - module_path: {:?}, name: {}, full_name: {}",
            module_path,
            name,
            full_name
        );

        let parsed_type = parser::parse_type(&full_name).map_err(|e| {
            if !known_bad_case(&full_name) {
                tracing::error!("Failed to parse type name `{full_name}`: {e}");
            }
            anyhow::anyhow!("Failed to parse type name `{full_name}`")
        })?;

        // let type_name = parsed_type.to_string();
        let typedef = parsed_type.as_typedef();

        tracing::debug!("TypeName::parse - name: {name}, typedef: {typedef:?}",);

        Ok(TypeName {
            module: ModuleName {
                segments: module_path.to_vec(),
            },
            name: typedef.display_name(),
            full_name,
            typedef,
        })
    }
}

impl fmt::Display for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.module.segments.is_empty() {
            write!(f, "{}::", self.module.segments.join("::"))?;
        }
        write!(f, "{}", self.full_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RawSymbol {
    name_bytes: Vec<u8>,
}

impl RawSymbol {
    pub fn new(name_bytes: Vec<u8>) -> Self {
        Self { name_bytes }
    }

    pub fn demangle(&self) -> anyhow::Result<SymbolName> {
        demangle_symbol(self.clone())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, salsa::Update)]
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

        let (module_path, lookup_name, hash) = parser::parse_symbol(path).map_err(|e| {
            // known bad cases we don't care about
            if !known_bad_case(path) {
                tracing::error!("Failed to parse symbol path `{path}`: {e}");
            }
            anyhow::anyhow!("Failed to parse symbol path `{path}`")
        })?;

        Ok(SymbolName {
            full_path: path.to_string(),
            lookup_name,
            hash: hash.unwrap_or_default(),
            module_path,
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

fn demangle_symbol(symbol: RawSymbol) -> anyhow::Result<SymbolName> {
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

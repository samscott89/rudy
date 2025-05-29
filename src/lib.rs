mod data;
mod database;
mod dwarf;
mod file;
mod formatting;
mod index;
mod outputs;
mod query;
mod types;

#[cfg(test)]
pub mod tests;

use anyhow::Result;
use data::TypeDef;
use std::fmt;

// reexport the public types from outputs
pub use outputs::{ResolvedAddress, ResolvedLocation, Type, Value, Variable};

pub struct DebugInfo {
    pub db: database::DebugDatabaseImpl,
}

impl fmt::Debug for DebugInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugInfo").finish()
    }
}

#[allow(dead_code)]
fn print_hex(data: &[u8]) -> String {
    let mut hex_string = String::new();
    for byte in data {
        hex_string.push_str(&format!("{:02x} ", byte));
    }
    hex_string
}

impl DebugInfo {
    pub fn new(binary_path: &str) -> Result<Self> {
        let db = database::DebugDatabaseImpl::new(binary_path)?;

        // load_eh_frame_info(binary_file, &data_arena)?;

        let pb = DebugInfo { db };
        Ok(pb)
    }

    pub fn resolve_position(
        &self,
        file: &str,
        line: u64,
        column: Option<u64>,
    ) -> Option<ResolvedAddress> {
        self.db.resolve_position(file, line, column).unwrap()
    }

    pub fn address_to_line(&self, address: u64) -> Option<ResolvedLocation> {
        self.db.resolve_address_to_location(address).unwrap()
    }

    pub fn resolve_function(&self, name: &str) -> Option<ResolvedAddress> {
        let f = self.db.lookup_function(name).unwrap()?;
        let address = f.function_body_address(&self.db);
        Some(ResolvedAddress { address })
    }

    pub fn get_source_lines(&self, _address: u64) -> Vec<String> {
        todo!()
    }

    pub fn resolve_variables_at_address(
        &self,
        address: u64,
        data_resolver: &dyn DataResolver,
    ) -> (Vec<Variable>, Vec<Variable>, Vec<Variable>) {
        let (locals, params, globals) = self
            .db
            .resolve_variables_at_address(address, data_resolver)
            .unwrap();
        (locals, params, globals)
    }

    pub fn test_get_shape(&self) -> TypeDef<'_> {
        self.db.test_get_shape().unwrap()
    }
}

pub trait DataResolver {
    fn base_address(&self) -> u64;
    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>>;
    fn read_address(&self, address: u64) -> Result<u64> {
        let data = self.read_memory(address, std::mem::size_of::<u64>())?;
        if data.len() != std::mem::size_of::<u64>() {
            return Err(anyhow::anyhow!("Failed to read address"));
        }
        let addr = u64::from_le_bytes(data.try_into().unwrap());
        tracing::trace!("read raw address: {addr:#x}");
        if addr == 0 {
            Ok(0)
        } else {
            addr.checked_sub(self.base_address())
                .ok_or_else(|| anyhow::anyhow!("Address underflow when adjusting for base address"))
        }
    }
    fn get_registers(&self) -> Result<Vec<u64>>;
    fn get_register(&self, idx: usize) -> Result<u64> {
        let registers = self.get_registers()?;
        registers
            .get(idx)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!("Invalid register index: {idx} (max: {})", registers.len())
            })
            .and_then(|addr| {
                // Adjust the address based on the base address
                addr.checked_sub(self.base_address()).ok_or_else(|| {
                    anyhow::anyhow!("Address underflow when adjusting for base address")
                })
            })
    }
}

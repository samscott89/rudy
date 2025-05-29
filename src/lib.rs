mod data;
mod database;
mod debug_info;
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

pub use debug_info::DebugInfo;
// reexport the public types from outputs
pub use outputs::{ResolvedAddress, ResolvedLocation, Type, Value, Variable};

#[allow(dead_code)]
fn print_hex(data: &[u8]) -> String {
    let mut hex_string = String::new();
    for byte in data {
        hex_string.push_str(&format!("{:02x} ", byte));
    }
    hex_string
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

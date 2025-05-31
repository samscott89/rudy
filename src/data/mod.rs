//! Functionality for introspecting Rust data types

mod read;
mod typedef;

pub use read::*;
pub use typedef::*;

use anyhow::Result;

/// Trait for resolving data from memory during debugging.
/// 
/// Implementors provide access to the target process's memory and registers,
/// allowing the debug info library to read variable values and follow pointers.
/// 
/// # Examples
/// 
/// ```no_run
/// use rust_debuginfo::DataResolver;
/// use anyhow::Result;
/// 
/// struct MyResolver {
///     base: u64,
///     // ... memory access implementation
/// }
/// 
/// impl DataResolver for MyResolver {
///     fn base_address(&self) -> u64 {
///         self.base
///     }
///     
///     fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>> {
///         // Read from target process memory
///         todo!()
///     }
///     
///     fn get_registers(&self) -> Result<Vec<u64>> {
///         // Get current register values
///         todo!()
///     }
/// }
/// ```
pub trait DataResolver {
    /// Returns the base address for memory calculations.
    /// 
    /// This is typically the base address where the binary is loaded in memory.
    /// All addresses returned by this trait should be adjusted by this base.
    fn base_address(&self) -> u64;
    
    /// Reads raw bytes from memory at the given address.
    /// 
    /// # Arguments
    /// 
    /// * `address` - The memory address to read from
    /// * `size` - Number of bytes to read
    /// 
    /// # Returns
    /// 
    /// The bytes read from memory
    fn read_memory(&self, address: u64, size: usize) -> Result<Vec<u8>>;
    
    /// Reads a 64-bit address from memory.
    /// 
    /// This method handles pointer dereferencing and base address adjustment.
    /// 
    /// # Arguments
    /// 
    /// * `address` - The memory address to read the pointer from
    /// 
    /// # Returns
    /// 
    /// The dereferenced address, adjusted for the base address
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
    /// Gets all register values from the target.
    /// 
    /// The order and meaning of registers is architecture-specific.
    /// 
    /// # Returns
    /// 
    /// A vector of register values
    fn get_registers(&self) -> Result<Vec<u64>>;
    
    /// Gets a specific register value by index.
    /// 
    /// # Arguments
    /// 
    /// * `idx` - The register index (architecture-specific)
    /// 
    /// # Returns
    /// 
    /// The register value, adjusted for the base address
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

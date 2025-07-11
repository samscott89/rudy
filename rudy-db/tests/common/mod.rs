mod definitions;

pub use ::test_utils::*;
pub use definitions::*;
use itertools::Itertools as _;
use rudy_db::DataResolver;

pub fn debug_db(target: Option<&'static str>) -> crate::DebugDb {
    crate::DebugDb::new().with_source_map(source_map(target))
}

#[macro_export]
macro_rules! function_name {
    () => {{
        // Okay, this is ugly, I get it. However, this is the best we can get on a stable rust.
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        // `3` is the length of the `::f`.
        &name[..name.len() - 3]
    }};
}

#[macro_export]
macro_rules! setup {
    ($($target:ident)?) => {{
        common::init_tracing();
        let mut settings = insta::Settings::clone_current();

        common::add_filters(&mut settings);

        // get current OS as a prefix
        $(
            settings.set_snapshot_suffix($target);
        )?
        settings.set_prepend_module_to_snapshot(false);

        let _guard = settings.bind_to_scope();
        let test_name = $crate::function_name!();
        let test_name = test_name
            .strip_prefix("rudy_db::")
            .unwrap_or(test_name);
        let test_name = test_name.strip_prefix("tests::").unwrap_or(test_name);
        let _span = tracing::info_span!("test", test_name, $($target)?).entered();
        (_guard, _span)
    }};
}

/// A DataResolver that reads from the current process memory
pub struct SelfProcessResolver {
    aslr_slide: u64,
}

impl SelfProcessResolver {
    pub fn new(aslr_slide: u64) -> Self {
        Self { aslr_slide }
    }
    pub fn file_to_runtime_address(&self, file_address: u64) -> u64 {
        file_address + self.aslr_slide
    }

    pub fn runtime_to_file_address(&self, runtime_address: u64) -> u64 {
        runtime_address - self.aslr_slide
    }
}

impl DataResolver for SelfProcessResolver {
    fn read_memory(&self, address: u64, size: usize) -> anyhow::Result<Vec<u8>> {
        if size > 4096 {
            return Err(anyhow::anyhow!("Attempting to read too much memory"));
        }
        // Read from our own process memory
        // This is safe because we're only reading memory we own
        let ptr = address as *const u8;
        let mut buffer = vec![0u8; size];

        unsafe {
            std::ptr::copy_nonoverlapping(ptr, buffer.as_mut_ptr(), size);
        }

        tracing::debug!(
            "Read {size} bytes from address {address:#016x}: {:?}",
            buffer
                .iter()
                .chunks(2)
                .into_iter()
                .map(|chunk| { chunk.map(|byte| format!("{byte:02x}")).collect::<String>() })
                .join(" ")
        );

        Ok(buffer)
    }

    fn get_register(&self, _idx: usize) -> anyhow::Result<u64> {
        // For testing, we need to provide dummy register values.
        Ok(0)
    }

    #[cfg(target_os = "linux")]
    fn get_stack_pointer(&self) -> anyhow::Result<u64> {
        // On Linux, we can read the stack pointer from the current thread's context
        use std::arch::asm;

        let sp: u64;
        unsafe {
            asm!("mov {}, rsp", out(reg) sp);
        }
        Ok(sp)
    }

    #[cfg(not(target_os = "linux"))]
    fn get_stack_pointer(&self) -> anyhow::Result<u64> {
        panic!("not supported on this platform");
    }
}

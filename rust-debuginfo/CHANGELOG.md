# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/samscott89/rust_debuginfo/releases/tag/rust-debuginfo-v0.0.1) - 2025-06-27

### Other

- Reading of BTreeMap values
- Get option/result working again
- New internal struct for btrees
- WIP btreemap parsing
- Enum parsing + reading
- More enum helper parsers
- Go back to more specialized option/result parsing
- Macro for tuples
- More parser stuff
- Re-org parser
- Refactor option parser to combinators
- combinators for hashmap impl
- More fun with combinators
- Children combinator working
- Bulk parsing
- Nice POC of using it
- POC parser combinator for DWARF
- More WIP...
- WIP
- Add (failing) btree introspection
- Fix tests
- BTree parsing
- Slice + better enum/option/result handling
- Some initial method discovery
- Finish field expressions
- More expression work
- WIP complex expressions
- Pull parser + type defs out of rust-debuginfo crate
- lazily index types, fix tests + run benchmarks
- Minor fixes
- More type resolution, pretty printing, hooked up with `rdi` command
- More hashmap support
- HashMap reading working
- Smart pointers
- Clean up error handling
- More type resolution + data reading
- Resolve options
- Inspect strings
- Working vec
- WIP reading from memory.
- Closer
- Make positions work again
- Addresses in a map
- slimmer, more efficient index
- Some nits
- Add some live introspection tests
- Hand roll a simple symbol parser
- More parsing
- More parsing
- More type handling
- Mostly just some nicer names in snapshots
- Refactoring
- Add some tests
- Parse TypeDef from names in dwarfinfo
- Remove lifetimes from typedefs
- Some pretty printing
- Implement parsing with `unsynn` for symbols + types
- A little more type parsing stuff
- Tests for std type detection
- getting std type resolution working
- Basic value printing working
- Move binaries to bin/ folder
- rdi-lldb basic flow
- Iniitial protocol
- Restructure repo to split rust-debuginfo lib + rdi-lldb bin crate

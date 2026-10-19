// Compose the real production module with package-dependent tests externally.
include!("../../../src/features/syntax/mod.rs");
#[cfg(test)]
#[path = "../suites/injection_tests.rs"]
mod injection_tests;
#[cfg(test)]
#[path = "../suites/test_support.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "../suites/treesitter_bench.rs"]
mod treesitter_bench;
#[cfg(test)]
#[path = "../suites/treesitter_tests.rs"]
mod treesitter_tests;

#[cfg(test)]
#[path = "../suites/presentation_integration_tests.rs"]
mod presentation_integration_tests;

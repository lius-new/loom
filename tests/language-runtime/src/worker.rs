include!("../../../src/infrastructure/syntax_worker.rs");
#[cfg(test)]
#[path = "../suites/worker_bench.rs"]
mod bench;
#[cfg(test)]
#[path = "../suites/worker_tests.rs"]
mod tests;

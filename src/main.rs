use std::any::Any;

/// Detect the panic raised by `println!`/`print!` when stdout is closed
/// (e.g. `bmpwatch dump file --jsonl | head -1`), so the process can exit
/// quietly like standard Unix tools instead of dumping a backtrace.
fn is_broken_pipe(payload: &(dyn Any + Send)) -> bool {
    let msg = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()));
    msg.is_some_and(|m| m.contains("Broken pipe"))
}

fn main() {
    let result = std::panic::catch_unwind(bmpwatch::cli::run);
    if let Err(payload) = result {
        if is_broken_pipe(payload.as_ref()) {
            std::process::exit(0);
        }
        std::panic::resume_unwind(payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    #[test]
    fn test_is_broken_pipe_detects_panic() {
        let payload = catch_unwind(AssertUnwindSafe(|| {
            panic!("failed printing to stdout: Broken pipe (os error 32)")
        }))
        .unwrap_err();
        assert!(is_broken_pipe(payload.as_ref()));
    }

    #[test]
    fn test_is_broken_pipe_ignores_other_panics() {
        let payload = catch_unwind(AssertUnwindSafe(|| panic!("some other bug"))).unwrap_err();
        assert!(!is_broken_pipe(payload.as_ref()));
    }
}

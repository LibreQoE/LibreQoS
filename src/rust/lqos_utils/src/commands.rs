/// `run_success` is a macro that wraps `std::process::Command`, and
/// obtains the status code. The macro returns `true` if the called
/// program returned success (0) and wasn't killed, and `false` if
/// anything went wrong.
/// 
/// # Examples
/// 
/// ```rust
/// use lqos_utils::run_success;
/// assert_eq!(run_success!("/bin/true"), true);
/// ```
/// 
/// ```rust
/// use lqos_utils::run_success;
/// assert!(run_success!("/bin/echo", "Hello World"));
/// assert!(run_success!("/bin/echo", "Hello", "World"));
/// ```
#[macro_export]
macro_rules! run_success {
    ($command:expr, $($arg:expr),*) => {
        {
            let status = std::process::Command::new($command)
                $(
                    .arg($arg)
                )*
                .status().unwrap();
            status.success()
        }
    };

    ($command: expr) => {
        {
            let status = std::process::Command::new($command)
                .status().unwrap();
            status.success()
        }
    };
}

/// Executes the `run_success` macro with the added caveat that it
/// will panic on failure. This is intended ONLY for times that running
/// the command successfully is absolutely critical.
/// 
/// ## Examples
/// 
/// ```rust
/// use lqos_utils::*;
/// run_or_panic!("/bin/true", !"It's not true");
/// ```
#[macro_export]
macro_rules! run_or_panic {
    ($command: expr, !$error: expr) => {
        if !run_success!($command) {
            panic!($error);
        }
    };

    ($command:expr, $($arg:expr),*, ?$error: expr) => {
        if !run_success!($command, $($arg),*) {
            panic!($error);
        }
    };
}

#[cfg(test)]
mod test {
    use crate::run_success;

    #[test]
    fn test_true() {
        assert!(run_success!("/bin/true"));
    }

    #[test]
    fn test_echo() {
        assert!(run_success!("/bin/echo", "Hello World"));
        assert!(run_success!("/bin/echo", "Hello", "World"));
    }

    #[test]
    fn test_expressions() {
        const ECHO: &str = "/bin/echo";
        const HELLO_WORLD: &str = "Hello World";
        assert!(run_success!(ECHO, HELLO_WORLD));
    }

    #[test]
    fn test_true_not_panic() {
        run_or_panic!("/bin/true", !"It's not true");
    }

    #[test]
    fn test_echo_not_panic() {
        run_or_panic!("/bin/echo", "Hello", "World", ?"Echo isn't working");
    }
}
//! `ck-occipital`: the supervised rich-UI teaching module, with optional tools.

#![forbid(unsafe_code)]
// The module runs agent-written programs and arguments. A panic would stop it
// serving every caller, so a broken internal assumption must become a typed
// error that the module reports as a tool outcome. Module code may not panic.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

// Calls into the language library must run inside `std::panic::catch_unwind`,
// so the module keeps serving after one bad program. With `panic = "abort"`
// that boundary would do nothing, so a build without unwinding is refused
// instead of shipping silently unprotected.
#[cfg(not(panic = "unwind"))]
compile_error!(
    "occipital-module requires panic = \"unwind\": its panic boundary does not work when panics abort"
);

fn main() -> std::process::ExitCode {
    // Read lossily: `std::env::args` panics on an argument that is not
    // UTF-8, and such an argument can only be a start that is not the
    // manifest probe. The SDK reads `--subc` from the raw arguments itself.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    occipital_module::startup::run(&args)
}

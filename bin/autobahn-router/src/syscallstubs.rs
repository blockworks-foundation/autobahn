use solana_program::program_stubs::{set_syscall_stubs, SyscallStubs};

// We don't want program output to spam out logs
struct NoLogSyscallStubs;
impl SyscallStubs for NoLogSyscallStubs {
    fn sol_log(&self, _message: &str) {
        // do nothing
        // TODO: optionally print it?
    }

    fn sol_log_data(&self, _fields: &[&[u8]]) {
        // do nothing
    }
}

pub fn deactivate_program_logs() {
    set_syscall_stubs(Box::new(NoLogSyscallStubs {}));
}

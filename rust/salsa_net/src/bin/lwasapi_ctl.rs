fn main() {
    salsa_wire::unix_sys::reset_sigpipe_default();
    salsa_wire::c_abi::run_c_main(salsa_wire::ctl_cli::salsa_ctl_main);
}

//! T-801 test plugin `gain` (see `vox_sandbox_ipc::test_plugins`).

fn main() -> std::process::ExitCode {
    vox_sandbox_ipc::test_plugins::main(vox_sandbox_ipc::test_plugins::TestPluginKind::Gain)
}

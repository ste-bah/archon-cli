use archon_core::config::ArchonConfig;

pub(crate) async fn handle_update_command(
    check: bool,
    force: bool,
    config: &ArchonConfig,
) -> anyhow::Result<()> {
    if check {
        match archon_core::update::check_update(&config.update).await {
            Ok(msg) => println!("{msg}"),
            Err(e) => eprintln!("update check failed: {e}"),
        }
    } else {
        match archon_core::update::perform_update(&config.update, force).await {
            Ok(msg) => println!("{msg}"),
            Err(archon_core::update::UpdateError::UpToDate(msg)) => println!("{msg}"),
            Err(e) => eprintln!("update failed: {e}"),
        }
    }
    Ok(())
}

//! Tauri entrypoint: builds the shell, manages `NodeService`, registers
//! commands, and applies the CSP from `tauri.conf.json`.

use minewrap_tauri::{NodeService, commands};

fn main() {
    let node_service = NodeService::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(node_service)
        .invoke_handler(tauri::generate_handler![
            commands::list_nodes,
            commands::list_instances,
            commands::get_snapshot,
            commands::instance_start,
            commands::instance_stop,
            commands::instance_restart,
            commands::send_command,
            commands::get_logs,
            commands::read_properties,
            commands::write_properties,
            commands::list_mods,
            commands::toggle_mod,
            commands::discord_test,
            commands::health,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Minewrap")
}

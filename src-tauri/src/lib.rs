pub mod agent;
pub mod settings;
pub mod terminal;

use settings::config::{self, ConfigState};
use terminal::{
    close_pty, get_completions, get_shell_history, resize_pty, spawn_pty, write_pty, PtyState,
};
use agent::ai_chat::{send_ai_message, continue_ai_with_tool_result, execute_tool_command, list_vertex_models, list_openrouter_models};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let initial_config = config::load_config();

    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                #[allow(unused_imports)]
                use tauri::Manager;
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .manage(PtyState::new())
        .manage(ConfigState::new(initial_config))
        .invoke_handler(tauri::generate_handler![
            spawn_pty,
            write_pty,
            resize_pty,
            close_pty,
            get_shell_history,
            get_completions,
            config::get_config,
            config::save_config,
            send_ai_message,
            continue_ai_with_tool_result,
            execute_tool_command,
            list_vertex_models,
            list_openrouter_models
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

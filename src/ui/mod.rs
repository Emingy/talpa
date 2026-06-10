mod controller;
mod tray;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::MenuEvent;

use tray::{Action, Ui};

/// User events injected into the tao event loop, all processed on the main
/// thread (where the UI lives). Menu clicks arrive on a background thread via
/// the muda handler; pipeline events are posted by the `Controller`'s spawned
/// start/stop tasks when they finish.
pub(crate) enum UserEvent {
    Menu(MenuEvent),
    /// A spawned start finished; payload is whether it succeeded.
    PipelineStarted(bool),
    /// A spawned stop finished.
    PipelineStopped,
    /// The background GitHub update check finished.
    UpdateChecked(crate::update::UpdateStatus),
}

/// Runs the status-bar app. Owns the main thread for the lifetime of the
/// process — the tao event loop never returns (it exits via `process::exit`).
pub fn run(config_path: String) {
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    // macOS: run as an "accessory" app so it lives only in the status bar — no
    // Dock icon, no app-switcher entry. Must be set before the loop runs.
    // (Windows/Linux create no window here, so there's no taskbar entry anyway.)
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    // Forward muda menu events into the tao loop so they wake it from `Wait`.
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    // A second proxy handed to the Controller so its spawned start/stop tasks
    // can post their results back into the loop.
    let ui_proxy = event_loop.create_proxy();

    // The tray must be created after NSApplication is up (StartCause::Init).
    let mut ui: Option<Ui> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {
                match Ui::new(config_path.clone(), ui_proxy.clone()) {
                    Ok(built) => ui = Some(built),
                    Err(e) => {
                        log::error!("[UI] failed to initialise tray: {e}");
                        std::process::exit(1);
                    }
                }
            }
            Event::UserEvent(UserEvent::Menu(menu_event)) => {
                if let Some(ui) = ui.as_mut()
                    && ui.handle_menu(&menu_event) == Action::Quit
                {
                    ui.shutdown();
                    std::process::exit(0);
                }
            }
            Event::UserEvent(UserEvent::PipelineStarted(ok)) => {
                if let Some(ui) = ui.as_mut() {
                    ui.on_pipeline_started(ok);
                }
            }
            Event::UserEvent(UserEvent::PipelineStopped) => {
                if let Some(ui) = ui.as_mut() {
                    ui.on_pipeline_stopped();
                }
            }
            Event::UserEvent(UserEvent::UpdateChecked(status)) => {
                if let Some(ui) = ui.as_mut() {
                    ui.on_update_checked(status);
                }
            }
            _ => {}
        }
    });
}

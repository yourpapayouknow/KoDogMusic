// 初始化窗口与业务线程。
use crate::{deps::*, view_models::AppWindow};

/// 启动窗口与通知区常驻事件循环。
pub fn runapp() -> Result<(), Box<dyn std::error::Error>> {
    BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select()?;
    let ui = AppWindow::new()?;
    ui.show()?;
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?).join("KoDogLite");
    let worker = crate::worker::Worker::nw(root);
    let timer = Rc::new(Timer::default());
    let controller = crate::bridge::bind(&ui, worker, timer.clone());
    let ctrl = controller.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || ctrl.borrow_mut().tick());
    run_event_loop_until_quit()?;
    timer.stop();
    drop(ui);
    drop(controller);
    Ok(())
}

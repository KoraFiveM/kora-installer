//! Showing a native notification. Clicking it asks the app to open the toast's page.

use kora_shell::toast::Toast;
use notify_rust::{Notification, NotificationResponse, Urgency};
use tao::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// Show `t`. A thread waits for the click, so nothing blocks the window; if the system cannot show
/// notifications at all, that is not an error worth stopping for.
pub fn show(t: Toast, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let mut n = Notification::new();
        n.summary(&t.title).body(&t.body).appname("Kora");
        #[cfg(windows)]
        {
            n.app_id(kora_shell::platform::AUMID);
        }
        #[cfg(not(windows))]
        {
            // freedesktop: the "default" action is the click on the notification itself.
            n.action("default", "Open Kora");
        }
        if t.urgent {
            n.urgency(Urgency::Critical);
        }
        let Ok(handle) = n.show() else { return };
        let route = t.route.clone();
        let _ = handle.wait_for_response(move |r: &NotificationResponse| {
            if matches!(r, NotificationResponse::Default | NotificationResponse::Action(_)) {
                let _ = proxy.send_event(UserEvent::OpenRoute(route.clone()));
            }
        });
    });
}

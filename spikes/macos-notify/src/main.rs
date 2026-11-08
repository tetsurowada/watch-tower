//! Spike: native macOS notification that opens a folder in Zed on click.
//!
//! Must run from inside a signed .app bundle (see bundle.sh).
//! Usage: macos-notify <folder> [title] [body]

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("this spike only runs on macOS");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AllocAnyThread};
    use objc2_foundation::{ns_string, NSBundle, NSDate, NSDictionary, NSError, NSRunLoop, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
        UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    static AUTH_DONE: AtomicBool = AtomicBool::new(false);
    static AUTH_GRANTED: AtomicBool = AtomicBool::new(false);
    static ADDED: AtomicBool = AtomicBool::new(false);
    static CLICKED: AtomicBool = AtomicBool::new(false);

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "WatchTowerSpikeDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            // Show banners even when this process counts as "in the foreground".
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                eprintln!("delegate: willPresent");
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Sound,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                handler: &DynBlock<dyn Fn()>,
            ) {
                let action = response.actionIdentifier();
                eprintln!("delegate: response action={action}");
                if &*action == unsafe { UNNotificationDefaultActionIdentifier } {
                    let info = response.notification().request().content().userInfo();
                    let key: &AnyObject = ns_string!("folder");
                    match info.objectForKey(key).and_then(|v| v.downcast::<NSString>().ok()) {
                        Some(folder) => open_in_zed(&folder.to_string()),
                        None => eprintln!("userInfo has no folder"),
                    }
                }
                CLICKED.store(true, Ordering::SeqCst);
                handler.call(());
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    fn open_in_zed(folder: &str) {
        eprintln!("running: open -a Zed {folder}");
        match Command::new("open").args(["-a", "Zed", folder]).status() {
            Ok(s) => eprintln!("open exited with {s}"),
            Err(e) => eprintln!("failed to run open: {e}"),
        }
    }

    /// Pumps the main run loop until `done` returns true or the timeout passes.
    fn pump_until(timeout: Duration, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let run_loop = NSRunLoop::mainRunLoop();
        while !done() {
            if Instant::now() >= deadline {
                return false;
            }
            run_loop.runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.1));
        }
        true
    }

    pub fn run() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let Some(folder) = args.first().cloned() else {
            eprintln!("usage: macos-notify <folder> [title] [body]");
            std::process::exit(2);
        };
        let title = args.get(1).cloned().unwrap_or_else(|| "watch-tower spike".into());
        let body = args
            .get(2)
            .cloned()
            .unwrap_or_else(|| format!("Click to open {folder} in Zed"));

        // UNUserNotificationCenter aborts the process without a bundle identifier.
        match NSBundle::mainBundle().bundleIdentifier() {
            Some(id) => eprintln!("bundle identifier: {id}"),
            None => {
                eprintln!("no bundle identifier: run the binary inside the .app (see bundle.sh)");
                std::process::exit(1);
            }
        }

        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = Delegate::new();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        let auth = RcBlock::new(|granted: Bool, error: *mut NSError| {
            if let Some(error) = unsafe { error.as_ref() } {
                eprintln!("authorization error: {error}");
            }
            AUTH_GRANTED.store(granted.as_bool(), Ordering::SeqCst);
            AUTH_DONE.store(true, Ordering::SeqCst);
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &auth,
        );
        if !pump_until(Duration::from_secs(60), || AUTH_DONE.load(Ordering::SeqCst)) {
            eprintln!("authorization did not finish within 60s");
            std::process::exit(1);
        }
        eprintln!("authorization granted: {}", AUTH_GRANTED.load(Ordering::SeqCst));
        if !AUTH_GRANTED.load(Ordering::SeqCst) {
            eprintln!("allow notifications for this app in System Settings > Notifications");
            std::process::exit(1);
        }

        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setSubtitle(ns_string!("session 1a2b3c4d · Stop"));
        content.setBody(&NSString::from_str(&body));
        content.setThreadIdentifier(ns_string!("spike-session"));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let folder_value = NSString::from_str(&folder);
        let info = NSDictionary::from_slices(&[ns_string!("folder")], &[&*folder_value]);
        unsafe { content.setUserInfo(Retained::cast_unchecked::<NSDictionary>(info).as_ref()) };

        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&format!("spike-{}", std::process::id())),
            &content,
            None,
        );
        let added = RcBlock::new(|error: *mut NSError| {
            match unsafe { error.as_ref() } {
                Some(error) => eprintln!("addNotificationRequest error: {error}"),
                None => eprintln!("notification added"),
            }
            ADDED.store(true, Ordering::SeqCst);
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&added));
        pump_until(Duration::from_secs(10), || ADDED.load(Ordering::SeqCst));

        eprintln!("waiting up to 120s for a click...");
        if pump_until(Duration::from_secs(120), || CLICKED.load(Ordering::SeqCst)) {
            eprintln!("done");
        } else {
            eprintln!("no click within 120s");
        }
    }
}

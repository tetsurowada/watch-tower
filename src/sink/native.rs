//! Native macOS notifications through UNUserNotificationCenter.
//!
//! Only works from inside a signed .app bundle (see the Makefile). Without a bundle
//! identifier UNUserNotificationCenter aborts the process, so `new` checks first.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSDate, NSError, NSRunLoop, NSString, NSURL};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAttachment,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use super::{Notification, Sink, State};

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "WatchTowerNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // Show banners even when macOS counts this process as frontmost.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

pub struct UserNotifications {
    resources: Option<PathBuf>,
    attachments: PathBuf,
}

impl UserNotifications {
    /// Returns None outside an app bundle, e.g. under `cargo run`.
    pub fn new() -> Option<Self> {
        let bundle = NSBundle::mainBundle();
        let id = bundle.bundleIdentifier()?;
        eprintln!("watch-tower: native notifications as {id}");

        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = Delegate::new();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center holds the delegate weakly; keep it for the life of the process.
        std::mem::forget(delegate);

        let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
            if let Some(error) = unsafe { error.as_ref() } {
                eprintln!("watch-tower: notification authorization error: {error}");
            }
            if !granted.as_bool() {
                eprintln!(
                    "watch-tower: notifications are off; turn them on for watch-tower in \
                     System Settings > Notifications"
                );
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &done,
        );

        Some(Self {
            resources: bundle.resourcePath().map(|p| PathBuf::from(p.to_string())),
            attachments: std::env::temp_dir().join("watch-tower-attachments"),
        })
    }

    fn attachment(&self, state: State) -> Option<Retained<UNNotificationAttachment>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let source = self.resources.as_ref()?.join(state.image());
        // The system moves an attached file into its own store, so attach a copy.
        let copy = self.attachments.join(format!(
            "{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            state.image()
        ));
        let copied =
            std::fs::create_dir_all(&self.attachments).and_then(|()| std::fs::copy(&source, &copy));
        if let Err(e) = copied {
            eprintln!("watch-tower: cannot attach {}: {e}", source.display());
            return None;
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&copy.to_string_lossy()));
        match unsafe {
            UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(
                &NSString::from_str("state"),
                &url,
                None,
            )
        } {
            Ok(attachment) => Some(attachment),
            Err(e) => {
                eprintln!("watch-tower: cannot attach {}: {e}", copy.display());
                let _ = std::fs::remove_file(&copy);
                None
            }
        }
    }
}

impl Sink for UserNotifications {
    fn send(&self, n: &Notification) -> Result<(), String> {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&n.title));
        content.setSubtitle(&NSString::from_str(&n.subtitle));
        content.setBody(&NSString::from_str(&n.message));
        if !n.thread.is_empty() {
            content.setThreadIdentifier(&NSString::from_str(&n.thread));
        }
        if !n.sound.is_empty() {
            // The Makefile copies the system sounds into the bundle's Resources.
            let name = NSString::from_str(&format!("{}.aiff", n.sound));
            content.setSound(Some(&UNNotificationSound::soundNamed(&name)));
        }
        if let Some(attachment) = n.state.and_then(|s| self.attachment(s)) {
            content.setAttachments(&NSArray::from_retained_slice(&[attachment]));
        }

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!(
            "watch-tower-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&id),
            &content,
            None,
        );
        let done = RcBlock::new(|error: *mut NSError| {
            if let Some(error) = unsafe { error.as_ref() } {
                eprintln!("watch-tower: notification failed: {error}");
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, Some(&done));
        Ok(())
    }
}

/// Runs the main run loop forever; delegate callbacks arrive through it.
pub fn run_main_loop() -> ! {
    let run_loop = NSRunLoop::mainRunLoop();
    loop {
        run_loop.runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(60.0));
    }
}

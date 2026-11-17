use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use futures_channel::oneshot;
use objc2::{AnyThread, define_class, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_user_notifications::{
    UNNotification, UNNotificationPresentationOptions, UNNotificationResponse,
    UNTextInputNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use crate::response::NotificationResponse;

static PENDING: OnceLock<Mutex<HashMap<String, oneshot::Sender<NotificationResponse>>>> =
    OnceLock::new();

fn pending() -> &'static Mutex<HashMap<String, oneshot::Sender<NotificationResponse>>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn register_response_sender(
    request_id: String,
    tx: oneshot::Sender<NotificationResponse>,
) {
    pending().lock().expect("pending map poisoned").insert(request_id, tx);
}

pub(crate) fn deregister_response_sender(request_id: &str) {
    let removed = pending().lock().expect("pending map poisoned").remove(request_id);
    if removed.is_some() {
        log::debug!("deregistered pending sender for {request_id:?}");
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MacUserNotificationsDelegate"]
    pub struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present_notification(
            &self,
            _center: &UNUserNotificationCenter,
            notification: &UNNotification,
            completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            let request_id = notification.request().identifier().to_string();
            log::debug!("willPresentNotification request_id={request_id:?}");

            completion_handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &block2::DynBlock<dyn Fn()>,
        ) {
            let request_id = response.notification().request().identifier().to_string();
            let action_id = response.actionIdentifier().to_string();

            log::debug!(
                "didReceiveNotificationResponse \
                 request_id={request_id:?} action={action_id:?}"
            );

            let reply_text: Option<String> = {
                let any: &AnyObject = response.as_ref();
                any.downcast_ref::<UNTextInputNotificationResponse>()
                    .map(|tr| tr.userText().to_string())
            };
            if let Some(ref text) = reply_text {
                log::debug!("reply text = {text:?}");
            }

            route_response(request_id, action_id, reply_text);

            completion_handler.call(());
        }
    }
);

pub(crate) fn route_response(request_id: String, action_id: String, reply_text: Option<String>) {
    let resp = NotificationResponse::from_objc(request_id.clone(), action_id, reply_text);
    crate::observer::dispatch(&resp);
    if let Some(tx) = pending().lock().expect("pending map poisoned").remove(&request_id) {
        if tx.send(resp).is_err() {
            log::warn!("receiver for request {request_id:?} was already dropped");
        }
    } else {
        log::debug!("no pending sender for request {request_id:?} (fire-and-forget notification)");
    }
}

impl NotificationDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { objc2::msg_send![super(this), init] }
    }
}

pub(crate) fn install() {
    static DELEGATE: OnceLock<Retained<NotificationDelegate>> = OnceLock::new();
    DELEGATE.get_or_init(|| {
        log::debug!("installing NotificationDelegate");
        let delegate = NotificationDelegate::new();
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
        delegate
    });
}

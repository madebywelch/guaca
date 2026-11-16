use std::time::Duration;
use tokio::sync::{oneshot, watch};

const DEADLINE: Duration = Duration::from_secs(5);
type Answer = oneshot::Sender<Result<Option<u64>, String>>;

pub(super) async fn complete(
    dispatch: impl FnOnce(Answer) -> Result<(), String>,
    mut hides: watch::Receiver<u64>,
) -> Result<(), String> {
    let (send, answer) = oneshot::channel();
    dispatch(send)?;
    tokio::time::timeout(DEADLINE, async {
        let closing = answer.await.map_err(|_| "the main thread dropped the menu bar toggle")??;
        if let Some(before) = closing {
            hides
                .wait_for(|epoch| *epoch != before)
                .await
                .map_err(|_| "the hide observer stopped")?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "the menu bar toggle did not complete in 5s".to_string())?
}

#[cfg(feature = "probe")]
pub(super) mod native {
    use super::*;
    use crate::tray::Tray;
    use std::sync::Arc;
    use tauri::{AppHandle, Manager};

    struct Hides(watch::Sender<u64>);

    pub fn install(app: &AppHandle) -> Result<(), String> {
        let (send, _) = watch::channel(0_u64);
        #[cfg(target_os = "macos")]
        {
            use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
            use objc2_app_kit::{NSApplication, NSApplicationDidHideNotification};
            use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
            use std::cell::RefCell;
            thread_local! {
                static OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> = const { RefCell::new(None) };
            }
            let main = MainThreadMarker::new().ok_or("the hide observer needs the main thread")?;
            let application = NSApplication::sharedApplication(main);
            let hides = send.clone();
            let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
                crate::probe::trace::event(crate::probe::trace::Phase::AppDidHide, 0, 0);
                hides.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
            });
            let observer = unsafe {
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(NSApplicationDidHideNotification),
                    Some(&application),
                    None,
                    &block,
                )
            };
            OBSERVER.with(|slot| *slot.borrow_mut() = Some(observer));
        }
        app.manage(Hides(send));
        Ok(())
    }

    pub async fn click(app: &AppHandle) -> Result<(), String> {
        let clock = app.state::<Hides>().0.clone();
        let hides = clock.subscribe();
        let target = app.clone();
        complete(
            |send| {
                app.run_on_main_thread(move || {
                    if send.is_closed() {
                        return;
                    }
                    let result = (|| {
                        let tray = target
                            .try_state::<Arc<Tray>>()
                            .ok_or("this launch has no menu bar icon")?;
                        #[cfg(target_os = "macos")]
                        let already_hidden = {
                            let main = objc2::MainThreadMarker::new()
                                .ok_or("the icon click needs the main thread")?;
                            objc2_app_kit::NSApplication::sharedApplication(main).isHidden()
                        };
                        #[cfg(not(target_os = "macos"))]
                        let already_hidden = true;
                        let before = *clock.borrow();
                        let closing = tray.probe_click()?;
                        Ok((closing && !already_hidden).then_some(before))
                    })();
                    let _ = send.send(result);
                })
                .map_err(|err| err.to_string())
            },
            hides,
        )
        .await?;
        crate::probe::trace::event(crate::probe::trace::Phase::ToggleCompleted, 0, 0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_waits_for_the_appkit_hide_after_the_toggle_returns() {
        let (hide, hides) = watch::channel(0);
        let done = complete(
            |send| {
                send.send(Ok(Some(0))).unwrap();
                Ok(())
            },
            hides,
        );
        tokio::pin!(done);
        use std::future::Future;
        std::future::poll_fn(|cx| {
            assert!(done.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        hide.send_replace(1);
        assert_eq!(done.await, Ok(()));
    }

    #[tokio::test]
    async fn a_hide_completed_inside_the_toggle_is_not_missed() {
        let (hide, hides) = watch::channel(3);
        assert_eq!(
            complete(
                |send| {
                    hide.send_replace(4);
                    send.send(Ok(Some(3))).unwrap();
                    Ok(())
                },
                hides
            )
            .await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn opening_or_an_already_hidden_app_needs_no_hide_event() {
        let (_hide, hides) = watch::channel(0);
        assert_eq!(
            complete(
                |send| {
                    send.send(Ok(None)).unwrap();
                    Ok(())
                },
                hides
            )
            .await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_failed_dispatch_is_returned_without_waiting() {
        let (_hide, hides) = watch::channel(0);
        assert_eq!(
            complete(|_| Err("dispatch refused".into()), hides).await,
            Err("dispatch refused".into())
        );
    }

    #[tokio::test]
    async fn a_failed_click_is_returned_without_waiting_for_hide() {
        let (_hide, hides) = watch::channel(0);
        assert_eq!(
            complete(
                |send| {
                    send.send(Err("no icon".into())).unwrap();
                    Ok(())
                },
                hides
            )
            .await,
            Err("no icon".into())
        );
    }

    #[tokio::test]
    async fn a_dropped_dispatch_is_an_error() {
        let (_hide, hides) = watch::channel(0);
        assert!(complete(|_| Ok(()), hides).await.unwrap_err().contains("dropped"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_missing_hide_is_a_bounded_error() {
        let (_hide, hides) = watch::channel(0);
        assert!(complete(
            |send| {
                send.send(Ok(Some(0))).unwrap();
                Ok(())
            },
            hides
        )
        .await
        .unwrap_err()
        .contains("5s"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_dispatch_times_out_and_cancels_the_queued_click() {
        let (_hide, hides) = watch::channel(0);
        let mut queued = None;
        assert!(complete(
            |send| {
                queued = Some(send);
                Ok(())
            },
            hides
        )
        .await
        .unwrap_err()
        .contains("5s"));
        assert!(queued.unwrap().is_closed());
    }

    #[tokio::test]
    async fn a_lost_hide_observer_is_an_error() {
        let (hide, hides) = watch::channel(0);
        drop(hide);
        assert!(complete(
            |send| {
                send.send(Ok(Some(0))).unwrap();
                Ok(())
            },
            hides
        )
        .await
        .unwrap_err()
        .contains("observer"));
    }
}

mod harness;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use parking_lot::Mutex;

use guac_lib::db::RelayState;
use guac_lib::domain::approval::ApprovalState;
use guac_lib::domain::ids::{ApprovalId, RunId};
use guac_lib::domain::now_ms;
use guac_lib::domain::place::{
    self, Abilities, AdapterId, Buttons, Card, CardKind, Heard, HeardKey, Mark, OutsideUser,
    PlaceKey, PostSafety, Posted, RelayTag, Said, Spot, ThreadKey,
};
use guac_lib::places::{Adapter, AdapterError, Ears, Pending, SendError};
use guac_lib::runtime::guard::GuardLimits;
use harness::*;

const DM: &str = "T1/D1";
const CHANNEL: &str = "T1/C1";
const PERSON: &str = "U1";
const STRANGER: &str = "U2";

#[derive(Debug, Clone)]
struct Post {
    spot: Spot,
    card: Card,
    tag: String,
}

#[derive(Default)]
struct World {
    abilities: Mutex<Option<Abilities>>,
    ears: Mutex<Option<Ears>>,
    posts: Mutex<Vec<Post>>,
    edits: Mutex<Vec<(Posted, Card)>>,
    marks: Mutex<Vec<(HeardKey, Mark)>>,
    tells: Mutex<Vec<(OutsideUser, String)>>,
    history: Mutex<Vec<Heard>>,
    caught: Mutex<Vec<(PlaceKey, Vec<ThreadKey>, i64)>>,
    failures: Mutex<VecDeque<SendError>>,
    hang_posts: AtomicBool,
    finds: AtomicUsize,
    hide_posts: AtomicBool,
    runs: AtomicUsize,
    ending: Mutex<Option<AdapterError>>,
}

impl World {
    fn new(abilities: Abilities) -> Arc<Self> {
        let world = Arc::new(World::default());
        *world.abilities.lock() = Some(abilities);
        world
    }

    fn posts(&self) -> Vec<Post> {
        self.posts.lock().clone()
    }

    fn told(&self, who: &str) -> Vec<String> {
        self.tells
            .lock()
            .iter()
            .filter(|(to, _)| to.0 == who)
            .map(|(_, text)| text.clone())
            .collect()
    }

    fn marks(&self) -> Vec<Mark> {
        self.marks.lock().iter().map(|(_, mark)| *mark).collect()
    }
}

struct Fake(Arc<World>);

impl Adapter for Fake {
    fn id(&self) -> AdapterId {
        adapter()
    }

    fn abilities(&self) -> Abilities {
        self.0.abilities.lock().expect("abilities are set")
    }

    fn run<'a>(&'a self, ears: Ears) -> Pending<'a, Result<(), AdapterError>> {
        async move {
            let ending = self.0.ending.lock().clone();
            if let Some(ending) = ending {
                self.0.runs.fetch_add(1, Ordering::SeqCst);
                return Err(ending);
            }
            ears.connected().await?;
            *self.0.ears.lock() = Some(ears);
            self.0.runs.fetch_add(1, Ordering::SeqCst);
            futures_util::future::pending::<()>().await;
            Ok(())
        }
        .boxed()
    }

    fn catch_up<'a>(
        &'a self,
        place: &'a PlaceKey,
        recent_threads: &'a [ThreadKey],
        since_ms: i64,
        ears: &'a Ears,
    ) -> Pending<'a, Result<(), AdapterError>> {
        async move {
            self.0.caught.lock().push((place.clone(), recent_threads.to_vec(), since_ms));
            let mut history = self.0.history.lock().clone();
            history.sort_by_key(|heard| heard.at_ms);
            for heard in history.into_iter().filter(|heard| {
                &heard.spot.place == place
                    && heard.at_ms >= since_ms
                    && heard
                        .spot
                        .thread
                        .as_ref()
                        .is_none_or(|thread| recent_threads.contains(thread))
            }) {
                ears.heard(heard)
                    .await
                    .map_err(|unheard| AdapterError { reason: unheard.reason, retry: true })?;
            }
            Ok(())
        }
        .boxed()
    }

    fn post<'a>(
        &'a self,
        to: &'a Spot,
        card: &'a Card,
        tag: &'a RelayTag,
    ) -> Pending<'a, Result<Posted, SendError>> {
        async move {
            if let Some(failure) = self.0.failures.lock().pop_front() {
                return Err(failure);
            }
            let posted = {
                let mut posts = self.0.posts.lock();
                posts.push(Post { spot: to.clone(), card: card.clone(), tag: tag.0.clone() });
                Posted(format!("p{}", posts.len()))
            };
            if self.0.hang_posts.load(Ordering::SeqCst) {
                futures_util::future::pending::<()>().await;
            }
            Ok(posted)
        }
        .boxed()
    }

    fn edit<'a>(
        &'a self,
        _to: &'a Spot,
        posted: &'a Posted,
        card: &'a Card,
    ) -> Pending<'a, Result<(), SendError>> {
        async move {
            self.0.edits.lock().push((posted.clone(), card.clone()));
            Ok(())
        }
        .boxed()
    }

    fn find<'a>(
        &'a self,
        _to: &'a Spot,
        tag: &'a RelayTag,
    ) -> Pending<'a, Result<Option<Posted>, SendError>> {
        async move {
            self.0.finds.fetch_add(1, Ordering::SeqCst);
            if self.0.hide_posts.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let posts = self.0.posts.lock();
            Ok(posts
                .iter()
                .position(|post| post.tag == tag.0)
                .map(|at| Posted(format!("p{}", at + 1))))
        }
        .boxed()
    }

    fn mark<'a>(
        &'a self,
        heard: &'a HeardKey,
        _to: &'a Spot,
        mark: Mark,
    ) -> Pending<'a, Result<(), SendError>> {
        async move {
            self.0.marks.lock().push((heard.clone(), mark));
            Ok(())
        }
        .boxed()
    }

    fn tell<'a>(
        &'a self,
        _to: &'a Spot,
        who: &'a OutsideUser,
        text: &'a str,
    ) -> Pending<'a, Result<(), SendError>> {
        async move {
            self.0.tells.lock().push((who.clone(), text.to_string()));
            Ok(())
        }
        .boxed()
    }
}

fn adapter() -> AdapterId {
    AdapterId("fake".into())
}

fn abilities() -> Abilities {
    Abilities {
        buttons: Some(Buttons { max: 5, label_chars: 30 }),
        edits: true,
        reactions: true,
        max_text: 4000,
        posts: PostSafety::Findable,
    }
}

fn said(key: &str, place: &str, thread: Option<&str>, speaker: &str, text: &str) -> Heard {
    Heard {
        key: HeardKey(key.into()),
        spot: Spot { place: PlaceKey(place.into()), thread: thread.map(|t| ThreadKey(t.into())) },
        direct: place == DM,
        speaker: OutsideUser(speaker.into()),
        at_ms: now_ms(),
        said: Said::Text { text: text.into() },
    }
}

fn press(key: &str, place: &str, speaker: &str, ask: ApprovalId, choice: &str) -> Heard {
    Heard {
        said: Said::Choice { ask: ask.to_string(), choice: choice.into() },
        ..said(key, place, None, speaker, "")
    }
}

async fn hear(world: &World, heard: Heard) {
    let ears = world.ears.lock().clone().expect("the adapter is running");
    ears.heard(heard).await.expect("heard");
}

struct Setup {
    h: Harness,
    world: Arc<World>,
}

async fn attach(h: &Harness, world: &Arc<World>) {
    let before = world.runs.load(Ordering::SeqCst);
    h.runtime.attach_adapter(Arc::new(Fake(world.clone())));
    h.wait_until("the adapter to connect", |_| world.runs.load(Ordering::SeqCst) > before).await;
}

async fn linked(stub: &Stub, abilities: Abilities) -> Setup {
    let h = harness_in_groups(
        stub,
        &[("Pip", Some("Home")), ("Ada", Some("Home"))],
        GuardLimits::default(),
    );
    let world = World::new(abilities);
    attach(&h, &world).await;
    let code = h.runtime.link_code(&adapter(), h.id("Pip")).unwrap();
    hear(&world, said("link", DM, None, PERSON, &code)).await;
    assert_eq!(world.told(PERSON), vec![place::linked("Pip")]);
    Setup { h, world }
}

fn run_for(h: &Harness, heard: &str) -> Option<RunId> {
    let conn = h.runtime.store().conn().unwrap();
    conn.query_row("SELECT run_id FROM place_runs WHERE heard_key=?1", [heard], |row| {
        row.get::<_, String>(0)
    })
    .ok()
    .map(|raw| raw.parse().unwrap())
}

fn runs(h: &Harness) -> i64 {
    let conn = h.runtime.store().conn().unwrap();
    conn.query_row("SELECT count(*) FROM place_runs", [], |row| row.get(0)).unwrap()
}

fn relay_state(h: &Harness, key: &str) -> Option<RelayState> {
    h.runtime.store().relay_row(key).unwrap().map(|row| row.state)
}

async fn settled(h: &Harness, heard: &str) -> RunId {
    h.wait_until("the message to become a run", |h| run_for(h, heard).is_some()).await;
    let run = run_for(h, heard).unwrap();
    h.settle(run).await;
    run
}

async fn relayed(h: &Harness, run: RunId, mark: Mark) {
    let key = format!("mark:{run}:{}", mark.as_str());
    h.wait_until(&format!("{key} to be relayed"), |h| {
        relay_state(h, &key).is_some_and(|state| !state.open())
    })
    .await;
}

fn texts(posts: &[Post]) -> Vec<String> {
    posts.iter().map(|post| post.card.text.clone()).collect()
}

async fn saying(text: &'static str) -> Stub {
    serve(move |_| Script::Say(text.into())).await
}

async fn asking() -> Stub {
    serve(|body| {
        if has_tool_result(body) {
            Script::Say("Shipping it.".into())
        } else {
            Script::AskQuestion {
                question: "Ship it?".into(),
                options: vec!["Ship it".into(), "Wait".into()],
            }
        }
    })
    .await
}

async fn hiring(done: &'static str) -> Stub {
    serve(move |body| {
        if has_tool_result(body) {
            Script::Say(done.into())
        } else {
            Script::Hire {
                name: "Chief of Product".into(),
                instructions: "You own the roadmap.".into(),
                notes: String::new(),
            }
        }
    })
    .await
}

async fn asked(h: &Harness) -> ApprovalId {
    h.awaited_request().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stranger_gets_one_line_and_nothing_is_stored_or_spent() {
    let stub = saying("Hello.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("s1", DM, None, STRANGER, "Do my taxes")).await;
    assert_eq!(world.told(STRANGER), vec![place::STRANGER]);
    assert_eq!(runs(&h), 0);
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
    assert!(!h.runtime.store().heard_before("outside:fake:s1").unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn before_anyone_links_everyone_is_a_stranger() {
    let stub = saying("Hello.").await;
    let h = harness_in_groups(&stub, &[("Pip", Some("Home"))], GuardLimits::default());
    let world = World::new(abilities());
    attach(&h, &world).await;
    hear(&world, said("s1", DM, None, PERSON, "Hi")).await;
    assert_eq!(world.told(PERSON), vec![place::STRANGER]);
    assert_eq!(runs(&h), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unbound_place_is_pointed_to_settings() {
    let stub = saying("Hello.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("u1", CHANNEL, Some("u1"), PERSON, "Anyone here?")).await;
    assert_eq!(world.told(PERSON).last().unwrap(), place::UNBOUND);
    assert_eq!(runs(&h), 0);
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_redelivered_message_makes_one_run_and_one_reply() {
    let stub = saying("Here you go.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    let heard = said("m1", DM, None, PERSON, "Summarize my week");
    world.history.lock().push(heard.clone());
    hear(&world, heard.clone()).await;
    hear(&world, heard.clone()).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    hear(&world, heard).await;
    attach(&h, &world).await;
    h.wait_until("catch-up to read past the message", |h| {
        h.runtime.store().high_water(&adapter(), &PlaceKey(DM.into())).unwrap()
            == Some(HeardKey("m1".into()))
    })
    .await;
    assert_eq!(runs(&h), 1);
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    assert_eq!(texts(&world.posts()), vec!["Here you go."]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_finds_a_missed_reply_in_an_old_thread_even_after_a_newer_live_event() {
    let stub = saying("Here.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    let old_root = "last-week";
    hear(&world, said("recent", DM, Some(old_root), PERSON, "Yesterday's follow-up")).await;
    settled(&h, "recent").await;
    let missed = said("missed", DM, Some(old_root), PERSON, "Earlier while disconnected");
    let live = said("live", DM, None, PERSON, "A newer message");
    hear(&world, live.clone()).await;
    settled(&h, "live").await;
    world.history.lock().extend([missed, live]);
    attach(&h, &world).await;
    settled(&h, "missed").await;
    let calls = world.caught.lock();
    assert!(calls.iter().any(|(place, roots, floor)| {
        place.0 == DM
            && roots == &[ThreadKey(old_root.into())]
            && now_ms() - floor >= place::STALE_MS
    }));
    assert_eq!(runs(&h), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_code_sent_in_a_channel_links_nothing() {
    let stub = saying("Hello.").await;
    let h = harness_in_groups(&stub, &[("Pip", Some("Home"))], GuardLimits::default());
    let world = World::new(abilities());
    attach(&h, &world).await;
    let code = h.runtime.link_code(&adapter(), h.id("Pip")).unwrap();
    hear(&world, said("c1", CHANNEL, None, PERSON, &code)).await;
    assert_eq!(world.told(PERSON), vec![place::CODE_IN_CHANNEL]);
    assert_eq!(h.runtime.store().linked(&adapter()).unwrap(), None);
    hear(&world, said("c2", DM, None, PERSON, &code)).await;
    hear(&world, said("c2", DM, None, PERSON, &code)).await;
    assert_eq!(runs(&h), 0);
    assert_eq!(world.told(PERSON).last().unwrap(), &place::linked("Pip"));
    assert_eq!(
        h.runtime.store().linked(&adapter()).unwrap().unwrap().person,
        OutsideUser(PERSON.into())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relinking_replaces_the_person() {
    let stub = saying("Hello.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    let code = h.runtime.link_code(&adapter(), h.id("Ada")).unwrap();
    hear(&world, said("relink", DM, None, STRANGER, &code)).await;
    assert_eq!(world.told(STRANGER), vec![place::linked("Ada")]);
    hear(&world, said("old", DM, None, PERSON, "still me?")).await;
    assert_eq!(world.told(PERSON).last().unwrap(), place::STRANGER);
}

async fn crash_after_post(posts: PostSafety) -> (Setup, String) {
    let stub = saying("The report is ready.").await;
    let setup = linked(&stub, Abilities { posts, ..abilities() }).await;
    let Setup { h, world } = &setup;
    world.hang_posts.store(true, Ordering::SeqCst);
    hear(world, said("m1", DM, None, PERSON, "Write the report")).await;
    h.wait_until("the reply to reach the provider", |_| world.posts().len() == 1).await;
    let key = world.posts()[0].tag.clone();
    assert_eq!(relay_state(h, &key), Some(RelayState::Sending));
    assert!(h.runtime.detach_adapter(&adapter()));
    world.hang_posts.store(false, Ordering::SeqCst);
    (setup, key)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crash_between_post_and_mark_finds_the_post_instead_of_posting_twice() {
    let (Setup { h, world }, key) = crash_after_post(PostSafety::Findable).await;
    attach(&h, &world).await;
    h.wait_until("the found post to be marked sent", |h| {
        relay_state(h, &key) == Some(RelayState::Sent)
    })
    .await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(world.finds.load(Ordering::SeqCst), 1);
    assert_eq!(texts(&world.posts()), vec!["The report is ready."]);
    assert_eq!(
        h.runtime.store().relay_row(&key).unwrap().unwrap().posted,
        Some(Posted("p1".into()))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_lookup_after_a_crash_keeps_the_post_in_doubt() {
    let (Setup { h, world }, key) = crash_after_post(PostSafety::Findable).await;
    world.hide_posts.store(true, Ordering::SeqCst);
    attach(&h, &world).await;
    h.wait_until("the uncertain post to be recorded", |h| {
        relay_state(h, &key) == Some(RelayState::Doubt)
    })
    .await;
    let row = h.runtime.store().relay_row(&key).unwrap().unwrap();
    assert!(row.last_error.unwrap().contains("may have reached"));
    assert_eq!(world.finds.load(Ordering::SeqCst), 1);
    assert_eq!(world.posts().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crash_between_post_and_mark_without_find_is_left_in_doubt_and_not_posted_again() {
    let (Setup { h, world }, key) = crash_after_post(PostSafety::Neither).await;
    attach(&h, &world).await;
    h.wait_until("the post to be in doubt", |h| relay_state(h, &key) == Some(RelayState::Doubt))
        .await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(world.finds.load(Ordering::SeqCst), 0);
    assert_eq!(world.posts().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crash_between_post_and_mark_on_an_idempotent_provider_posts_again_with_the_same_tag() {
    let (Setup { h, world }, key) = crash_after_post(PostSafety::Idempotent).await;
    attach(&h, &world).await;
    h.wait_until("the post to be sent", |h| relay_state(h, &key) == Some(RelayState::Sent)).await;
    let posts = world.posts();
    assert_eq!(world.finds.load(Ordering::SeqCst), 0);
    assert_eq!(posts.len(), 2);
    assert!(posts.iter().all(|post| post.tag == key));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rate_limit_is_waited_out_and_the_reply_still_arrives_once() {
    let stub = saying("Done and dusted.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    world.failures.lock().push_back(SendError::Retry { after: Some(Duration::from_millis(200)) });
    hear(&world, said("m1", DM, None, PERSON, "Do it")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    let message = world.posts()[0].tag.clone();
    h.wait_until("the reply to be sent", |h| relay_state(h, &message) == Some(RelayState::Sent))
        .await;
    assert_eq!(texts(&world.posts()), vec!["Done and dusted."]);
    assert_eq!(h.runtime.store().relay_row(&message).unwrap().unwrap().attempts, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_post_fails_its_row_with_the_reason_and_the_spot_moves_on() {
    let stub = saying("Here.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    world
        .failures
        .lock()
        .push_back(SendError::Refused { reason: "Slack refused the message: msg_too_long".into() });
    hear(&world, said("m1", DM, None, PERSON, "Go")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    let message = h.runtime.store().replies_in_run(run).unwrap().remove(0);
    let reply = h
        .runtime
        .store()
        .relay_row(&format!("message:{}", message.id))
        .unwrap()
        .expect("the reply has a row");
    assert_eq!(reply.state, RelayState::Failed);
    assert_eq!(reply.last_error.as_deref(), Some("Slack refused the message: msg_too_long"));
    assert!(world.posts().is_empty());
    assert_eq!(world.marks(), vec![Mark::Working, Mark::Done]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pieces_of_a_long_reply_keep_their_order_through_retries() {
    let stub = saying("alpha beta gamma delta epsilon zeta eta theta").await;
    let Setup { h, world } = linked(&stub, Abilities { max_text: 12, ..abilities() }).await;
    {
        let mut failures = world.failures.lock();
        failures.push_back(SendError::Retry { after: Some(Duration::from_millis(100)) });
        failures.push_back(SendError::Retry { after: Some(Duration::from_millis(100)) });
    }
    hear(&world, said("m1", DM, None, PERSON, "Greek")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    h.wait_until("every piece to be posted", |_| world.posts().len() == 4).await;
    assert_eq!(
        texts(&world.posts()),
        vec!["alpha beta", "gamma delta", "epsilon zeta", "eta theta"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_press_after_the_app_answered_changes_nothing_and_the_card_shows_what_happened() {
    let stub = asking().await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("m1", DM, None, PERSON, "Release?")).await;
    let ask = asked(&h).await;
    h.wait_until("the question to be posted", |_| {
        world.posts().iter().any(|p| p.card.kind == CardKind::Question)
    })
    .await;
    h.runtime.answer_question(ask, "Wait").unwrap();
    hear(&world, press("press1", DM, PERSON, ask, "0")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    h.wait_until("the card to be edited", |_| !world.edits.lock().is_empty()).await;
    let approval = h.runtime.store().get_approval(ask).unwrap().unwrap();
    assert_eq!(approval.answer.as_deref(), Some("Wait"));
    let edits = world.edits.lock().clone();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].1.settled.as_deref(), Some("Answered: Wait"));
    assert!(edits[0].1.choices.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_press_from_a_stranger_decides_nothing() {
    let stub = asking().await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("m1", DM, None, PERSON, "Release?")).await;
    let ask = asked(&h).await;
    hear(&world, press("press1", DM, STRANGER, ask, "0")).await;
    assert_eq!(world.told(STRANGER), vec![place::STRANGER]);
    assert_eq!(h.runtime.store().get_approval(ask).unwrap().unwrap().state, ApprovalState::Pending);
    hear(&world, press("press2", DM, PERSON, ask, "0")).await;
    assert_eq!(
        h.runtime.store().get_approval(ask).unwrap().unwrap().answer.as_deref(),
        Some("Ship it")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_press_on_a_request_not_asked_through_this_adapter_is_refused() {
    let stub = asking().await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    h.runtime.send_from_human(h.id("Pip"), "Release?").unwrap();
    let ask = asked(&h).await;
    hear(&world, press("press1", DM, PERSON, ask, "0")).await;
    assert_eq!(world.told(PERSON).last().unwrap(), place::NOT_FROM_HERE);
    assert_eq!(h.runtime.store().get_approval(ask).unwrap().unwrap().state, ApprovalState::Pending);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turn_drained_for_an_update_still_answers_the_place_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let stub = serve(move |_| match counted.fetch_add(1, Ordering::SeqCst) {
        0 => Script::Hang,
        _ => Script::Say("Picked it back up.".into()),
    })
    .await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("m1", DM, None, PERSON, "Keep going")).await;
    h.wait_until("the model call to be in the air", |_| calls.load(Ordering::SeqCst) >= 1).await;
    h.runtime.detach_adapter(&adapter());
    let (h, drained) = h.restart(Duration::from_secs(5)).await;
    assert!(drained.is_complete(), "{drained:?}");
    attach(&h, &world).await;
    let run = run_for(&h, "m1").unwrap();
    h.settle(run).await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(texts(&world.posts()), vec!["Picked it back up."]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_heard_while_the_host_drains_is_put_down_with_its_place() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let stub = serve(move |body| {
        if speaker(body) == "Ada" {
            return match counted.fetch_add(1, Ordering::SeqCst) {
                0 => Script::Hang,
                _ => Script::Say("Ada here.".into()),
            };
        }
        Script::Say("Pip here.".into())
    })
    .await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    h.runtime.send_from_human(h.id("Ada"), "Think slowly").unwrap();
    h.wait_until("Ada's model call to be in the air", |_| calls.load(Ordering::SeqCst) >= 1).await;
    let draining = h.runtime.clone();
    let drain = tokio::spawn(async move { draining.drain(Duration::from_secs(2)).await });
    h.wait_until("the host to drain", |h| h.runtime.draining()).await;
    hear(&world, said("m1", DM, None, PERSON, "Are you there?")).await;
    let run = run_for(&h, "m1").expect("the place run rode in the put-down commit");
    drain.await.unwrap();
    h.runtime.detach_adapter(&adapter());
    let (h, _) = h.restart(Duration::from_secs(5)).await;
    attach(&h, &world).await;
    h.settle(run).await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(texts(&world.posts()), vec!["Pip here."]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stopped_run_posts_a_stop_notice_instead_of_a_done_mark() {
    let stub = serve(|_| Script::Hang).await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("m1", DM, None, PERSON, "Take your time")).await;
    h.wait_until("the run to start", |h| run_for(h, "m1").is_some()).await;
    let run = run_for(&h, "m1").unwrap();
    h.wait_until("the turn to be in the air", |_| stub.calls.load(Ordering::SeqCst) >= 1).await;
    assert!(h.runtime.stop_run(run));
    h.settle(run).await;
    h.wait_until("the stop notice", |h| {
        relay_state(h, &format!("notice:{run}:stopped")) == Some(RelayState::Sent)
    })
    .await;
    assert_eq!(texts(&world.posts()), vec!["Guaca stopped this run."]);
    assert_eq!(world.marks(), vec![Mark::Working]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_older_than_a_day_gets_one_line_and_no_run() {
    let stub = saying("Hello.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    let old = Heard {
        at_ms: now_ms() - place::STALE_MS - 60_000,
        ..said("old", DM, None, PERSON, "Book the room")
    };
    hear(&world, old.clone()).await;
    hear(&world, old).await;
    h.wait_until("the line to be posted", |_| !world.posts().is_empty()).await;
    let posts = world.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].card.from, None);
    assert_eq!(posts[0].card.kind, CardKind::Notice);
    assert!(posts[0].card.text.contains("\"Book the room\""), "{}", posts[0].card.text);
    assert!(posts[0].card.text.contains("Ask again if it still matters."));
    assert_eq!(runs(&h), 0);
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_answer_to_a_message_from_while_guaca_was_off_is_introduced_by_guaca() {
    let stub = saying("Booked.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    let late =
        Heard { at_ms: now_ms() - 10 * 60_000, ..said("late", DM, None, PERSON, "Book the room") };
    hear(&world, late).await;
    let run = settled(&h, "late").await;
    relayed(&h, run, Mark::Done).await;
    h.wait_until("both posts", |_| world.posts().len() == 2).await;
    let posts = world.posts();
    assert_eq!(posts[0].card.from, None);
    assert!(posts[0].card.text.contains("Guaca was offline when you sent \"Book the room\""));
    assert_eq!(posts[1].card.from.as_deref(), Some("Pip"));
    assert_eq!(posts[1].card.text, "Booked.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fresh_message_is_answered_without_an_offline_line() {
    let stub = saying("Booked.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("now", DM, None, PERSON, "Book the room")).await;
    let run = settled(&h, "now").await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(texts(&world.posts()), vec!["Booked."]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_permission_asked_from_a_channel_goes_to_the_direct_place_and_the_channel_is_told() {
    let stub = hiring("Sent it.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    let pip = h.runtime.store().get_agent(h.id("Pip")).unwrap().unwrap();
    h.runtime
        .store()
        .bind_place(&adapter(), &PlaceKey(CHANNEL.into()), pip.group_id, pip.id, false)
        .unwrap();
    hear(&world, said("m1", CHANNEL, Some("m1"), PERSON, "Send the draft to the board")).await;
    let ask = asked(&h).await;
    h.wait_until("the request and the waiting line", |_| world.posts().len() == 2).await;
    let posts = world.posts();
    let request =
        posts.iter().find(|p| p.card.kind == CardKind::Permission).expect("the request was posted");
    assert_eq!(request.spot, Spot { place: PlaceKey(DM.into()), thread: None });
    let waiting =
        posts.iter().find(|p| p.card.kind == CardKind::Notice).expect("the channel was told");
    assert_eq!(waiting.spot.place, PlaceKey(CHANNEL.into()));
    assert_eq!(waiting.card.text, place::waiting_for_owner("Pip"));
    hear(&world, press("press1", DM, PERSON, ask, place::ALLOW)).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(h.runtime.store().get_approval(ask).unwrap().unwrap().state, ApprovalState::Allow);
    let edits = world.edits.lock().clone();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].1.settled.as_deref(), Some("Allowed"));
    let channel: Vec<String> = world
        .posts()
        .iter()
        .filter(|p| p.spot.place.0 == CHANNEL)
        .map(|p| p.card.text.clone())
        .collect();
    assert_eq!(channel, vec![place::waiting_for_owner("Pip"), "Sent it.".into()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_direct_message_is_answered_by_the_default_agent_with_marks() {
    let stub = saying("Morning.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("m1", DM, None, PERSON, "Good morning")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    let posts = world.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].card.kind, CardKind::Reply);
    assert_eq!(posts[0].card.from.as_deref(), Some("Pip"));
    assert_eq!(posts[0].spot, Spot { place: PlaceKey(DM.into()), thread: None });
    assert_eq!(world.marks(), vec![Mark::Working, Mark::Done]);
    assert_eq!(world.marks.lock()[0].0, HeardKey("m1".into()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_leading_name_picks_the_agent_and_the_thread_stays_with_it() {
    let stub = serve(|body| Script::Say(format!("{} speaking.", speaker(body)))).await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    let pip = h.runtime.store().get_agent(h.id("Pip")).unwrap().unwrap();
    h.runtime
        .store()
        .bind_place(&adapter(), &PlaceKey(CHANNEL.into()), pip.group_id, pip.id, false)
        .unwrap();
    hear(&world, said("t1", CHANNEL, Some("t1"), PERSON, "@ada what is due?")).await;
    let first = settled(&h, "t1").await;
    relayed(&h, first, Mark::Done).await;
    hear(&world, said("t2", CHANNEL, Some("t1"), PERSON, "and after that?")).await;
    let second = settled(&h, "t2").await;
    relayed(&h, second, Mark::Done).await;
    hear(&world, said("t3", CHANNEL, Some("t3"), PERSON, "and you?")).await;
    let third = settled(&h, "t3").await;
    relayed(&h, third, Mark::Done).await;
    assert_eq!(texts(&world.posts()), vec!["Ada speaking.", "Ada speaking.", "Pip speaking."]);
    let first_message = h.runtime.store().channel_messages(h.id("Ada"), 10).unwrap();
    assert!(first_message.iter().any(|m| m.plain_text() == "what is due?"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_buttons_a_question_is_numbered_text_and_a_number_answers_it() {
    let stub = asking().await;
    let Setup { h, world } = linked(&stub, Abilities { buttons: None, ..abilities() }).await;
    hear(&world, said("m1", DM, None, PERSON, "Release?")).await;
    let ask = asked(&h).await;
    h.wait_until("the question to be posted", |_| {
        world.posts().iter().any(|p| p.card.kind == CardKind::Question)
    })
    .await;
    let question = world.posts().into_iter().find(|p| p.card.kind == CardKind::Question).unwrap();
    assert!(question.card.choices.is_empty());
    assert!(question.card.text.contains("1. Ship it\n2. Wait"), "{}", question.card.text);
    hear(&world, said("m2", DM, None, PERSON, "2")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(
        h.runtime.store().get_approval(ask).unwrap().unwrap().answer.as_deref(),
        Some("Wait")
    );
    assert_eq!(runs(&h), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn free_text_in_the_spot_of_a_pending_question_answers_it_once_even_when_redelivered() {
    let stub = asking().await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("m1", DM, None, PERSON, "Release?")).await;
    let ask = asked(&h).await;
    h.wait_until("the question to be posted", |_| {
        world.posts().iter().any(|p| p.card.kind == CardKind::Question)
    })
    .await;
    let answer = said("m2", DM, None, PERSON, "after lunch");
    hear(&world, answer.clone()).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    hear(&world, answer).await;
    assert_eq!(
        h.runtime.store().get_approval(ask).unwrap().unwrap().answer.as_deref(),
        Some("after lunch")
    );
    assert_eq!(runs(&h), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_edits_a_settled_ask_is_a_short_follow_up() {
    let stub = asking().await;
    let Setup { h, world } = linked(&stub, Abilities { edits: false, ..abilities() }).await;
    hear(&world, said("m1", DM, None, PERSON, "Release?")).await;
    let ask = asked(&h).await;
    h.wait_until("the question to be posted", |_| {
        world.posts().iter().any(|p| p.card.kind == CardKind::Question)
    })
    .await;
    hear(&world, press("press1", DM, PERSON, ask, "0")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    h.wait_until("the follow-up", |_| world.posts().len() == 3).await;
    assert!(world.edits.lock().is_empty());
    assert_eq!(
        texts(&world.posts())[1..],
        ["Answered: Ship it".to_string(), "Shipping it.".to_string()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_reactions_marks_are_skipped_and_a_failure_is_a_notice() {
    let stub = serve(|_| Script::Unavailable).await;
    let Setup { h, world } = linked(&stub, Abilities { reactions: false, ..abilities() }).await;
    hear(&world, said("m1", DM, None, PERSON, "Try")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Failed).await;
    h.wait_until("the failure notice", |_| {
        world.posts().iter().any(|p| p.card.text.ends_with(place::TRY_AGAIN))
    })
    .await;
    assert!(world.marks().is_empty());
    assert_eq!(relay_state(&h, &format!("mark:{run}:failed")), Some(RelayState::Skipped));
    let notice = world.posts().into_iter().find(|p| p.card.kind == CardKind::Notice).unwrap();
    assert_eq!(notice.card.from, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_adapter_that_ends_for_good_stays_stopped_until_attached_again() {
    let stub = saying("Hello.").await;
    let h = harness_in_groups(&stub, &[("Pip", Some("Home"))], GuardLimits::default());
    let world = World::new(abilities());
    *world.ending.lock() =
        Some(AdapterError { reason: "The token was revoked.".into(), retry: false });
    attach(&h, &world).await;
    h.wait_until("the adapter to stop", |h| {
        h.runtime.adapter_status(&adapter()).is_some_and(|s| s.stopped)
    })
    .await;
    let status = h.runtime.adapter_status(&adapter()).unwrap();
    assert_eq!(status.last_error.as_deref(), Some("The token was revoked."));
    assert_eq!(status.connected_since, None);
    assert_eq!(world.runs.load(Ordering::SeqCst), 1);
    *world.ending.lock() = None;
    attach(&h, &world).await;
    assert!(!h.runtime.adapter_status(&adapter()).unwrap().stopped);
    assert!(h.runtime.adapter_status(&adapter()).unwrap().connected_since.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_number_never_grants_a_permission_posted_with_buttons() {
    let stub = hiring("Understood.").await;
    let Setup { h, world } = linked(&stub, abilities()).await;
    hear(&world, said("m1", DM, None, PERSON, "Hire an agent")).await;
    let ask = asked(&h).await;
    h.wait_until("the request to be sent", |h| {
        relay_state(h, &format!("ask:{ask}")) == Some(RelayState::Sent)
    })
    .await;
    hear(&world, said("m2", DM, None, PERSON, "1")).await;
    let approval = h.runtime.store().get_approval(ask).unwrap().unwrap();
    assert_eq!(approval.state, ApprovalState::Pending);
    assert!(run_for(&h, "m2").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_decision_typed_as_a_number_settles_a_permission() {
    let stub = hiring("Understood.").await;
    let Setup { h, world } = linked(&stub, Abilities { buttons: None, ..abilities() }).await;
    hear(&world, said("m1", DM, None, PERSON, "Email the board")).await;
    let ask = asked(&h).await;
    h.wait_until("the request to be sent", |h| {
        relay_state(h, &format!("ask:{ask}")) == Some(RelayState::Sent)
    })
    .await;
    hear(&world, said("m2", DM, None, PERSON, "3")).await;
    let run = settled(&h, "m1").await;
    relayed(&h, run, Mark::Done).await;
    assert_eq!(h.runtime.store().get_approval(ask).unwrap().unwrap().state, ApprovalState::Deny);
    assert!(h.agent_named("Chief of Product").is_none());
}

//! Apple Container, against the real runtime and the real image.
//!
//! Every claim `computer/apple.rs` makes about what the `container` CLI prints
//! was read from Apple's 1.2.2 documentation and sources on a Mac that has no
//! `container` on it. This suite is where those claims meet the binary, and
//! where the desktop image meets the commands `computer/desktop.rs` runs on it.
//! It is the ten smoke items in `docs/LOCAL_COMPUTERS.md` under "Provider smoke
//! tests", one test each, in that order.
//!
//! Every test is `#[ignore]`d: it needs Apple Container installed, its service
//! running, and a built desktop image. `scripts/spike-apple.sh` arranges all
//! three and runs this. Nothing here is part of `./scripts/ci.sh`, which has no
//! runtime to talk to.
//!
//! Two things are asserted with more force than the rest, because their failure
//! is silent and destructive rather than a red test:
//!
//! - `list_owned` must return the container's *name*. The sweep matches what it
//!   returns against `provider_id` on the rows, so if `configuration.id` holds
//!   anything else, every live machine looks unclaimed and the first sweep after
//!   a restart deletes all of them.
//! - A second agent must not reach the first's noVNC port. That is a release
//!   blocker in the spec, and it is asserted against a port that is genuinely
//!   serving, with a control probe from inside the same machine, because a
//!   refusal from a dead port proves nothing at all.
//!
//! Anything a test creates is deleted by a guard's `Drop`, including when the
//! test ends by panicking: a leftover holds 20 GiB and a network, and the next
//! run would meet its own leftovers instead of a clean Mac.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use guac_lib::computer::apple::AppleContainer;
use guac_lib::computer::cli::{Cli, CliOutput};
use guac_lib::computer::desktop::{decode_bytes, encode, DesktopAction};
use guac_lib::computer::provider::{
    ComputerProvider, CreateComputer, ExecRequest, ProviderError, ProviderHandle, ProviderState,
    ViewerTarget,
};
use guac_lib::computer::Machine;
use guac_lib::domain::ids::{AgentId, ComputerId};
use guac_lib::proxy::ViewerResolver;
// The real thing rather than a copy of it: a suite that carried its own idea of
// how big a write is passed while the app failed on the same guest.
use guac_lib::runtime::{place_command, INBOX, MAX_GUEST_ARG, PLACE_CHUNK};

/// The installation label everything this suite makes carries. Its own, so
/// `list_owned` here answers about this suite and never about a machine the app
/// itself is running on the same Mac.
const INSTALLATION: &str = "apple-spike";

/// Where noVNC serves inside the guest. Spelled here because
/// `desktop::VNC_PORT` is crate-internal, and because this is the number the
/// isolation test asks a second machine to fail to reach.
const VNC_PORT: u16 = 6080;

/// Long enough for a first boot with an XFCE session in it.
const SETTLE: Duration = Duration::from_secs(180);

/// A value that must be in exactly one command's environment and nowhere else.
/// Not a real credential, and deliberately shaped like one.
const SENTINEL: &str = "spike-sentinel-9f21c7-not-a-secret";

/// The runtime, or a failure that says what is missing.
///
/// Deliberately not a skip. Every test here is already behind `#[ignore]`, so
/// nothing reaches this without somebody asking for it by name, and a suite
/// that answered "10 passed" on a Mac with no runtime would be a conformance
/// report for a machine that was never made — read, reasonably, as the spike
/// having been done.
fn runtime() -> Arc<AppleContainer> {
    match AppleContainer::discover(INSTALLATION) {
        Some(provider) => Arc::new(provider),
        None => panic!(
            "Apple Container is not installed: no `container` binary at \
             /usr/local/bin/container or on PATH. Install the signed package from \
             github.com/apple/container/releases, then run scripts/spike-apple.sh."
        ),
    }
}

/// The runtime's own words, for the questions the provider deliberately does
/// not ask: whether a volume still exists, and everything `inspect` prints.
async fn container(argv: &[&str]) -> CliOutput {
    let cli = Cli::discover("container", &["/usr/local/bin/container"])
        .expect("the suite only runs when `container` is installed");
    cli.run(
        &argv.iter().map(|part| part.to_string()).collect::<Vec<_>>(),
        &BTreeMap::new(),
        Duration::from_secs(60),
    )
    .await
    .unwrap_or_else(|err| panic!("`container {}` could not be run: {err}", argv.join(" ")))
}

/// One machine, released when the test that made it ends.
struct Spike {
    provider: Arc<AppleContainer>,
    handle: ProviderHandle,
    machine: Machine,
    computer: ComputerId,
}

impl Spike {
    /// A machine with the app's own idle period, which is long enough that
    /// nothing sleeps underneath a test.
    async fn make(provider: &Arc<AppleContainer>) -> Spike {
        Spike::with_idle(provider, 900, 0).await
    }

    /// `idle_seconds` is the number the image's watchdog reads, so the sleep
    /// test sets it low; `viewer_port` is only consulted by `Machine::vnc_url`.
    async fn with_idle(
        provider: &Arc<AppleContainer>,
        idle_seconds: u32,
        viewer_port: u16,
    ) -> Spike {
        let computer = ComputerId::new();
        let handle = provider
            .create(&CreateComputer {
                computer,
                agent: AgentId::new(),
                agent_name: "spike".into(),
                idle_seconds,
            })
            .await
            .unwrap_or_else(|err| panic!("a machine to work on: {err}"));

        let machine = Machine::new(
            provider.clone() as Arc<dyn ComputerProvider>,
            handle.clone(),
            BTreeMap::new(),
            viewer_port,
        );
        Spike { provider: provider.clone(), handle, machine, computer }
    }

    /// One command, insisting on success, said the way the runtime says it.
    async fn run(&self, command: &str) -> String {
        let out = self
            .machine
            .run_plain(command)
            .await
            .unwrap_or_else(|err| panic!("could not run on the machine: {err}"));
        assert_eq!(
            out.exit_code, 0,
            "command failed: {command}\nstdout: {}\nstderr: {}",
            out.stdout, out.stderr
        );
        out.stdout
    }

    /// The guest's address on its own network, which is what the viewer proxy
    /// connects to and what a second agent must not be able to reach.
    async fn address(&self) -> String {
        self.provider
            .viewer_target(&self.handle, VNC_PORT)
            .await
            .unwrap_or_else(|err| panic!("the machine has no address yet: {err}"))
            .host
    }

    /// Waits for a state rather than assuming one: a container that has been
    /// asked to stop is not stopped yet, and a boot is not instant.
    async fn settles_at(&self, want: ProviderState, within: Duration) {
        let deadline = Instant::now() + within;
        let mut last = None;
        while Instant::now() < deadline {
            match self.provider.inspect(&self.handle).await {
                Ok(state) if state == want => return,
                Ok(state) => last = Some(format!("{state:?}")),
                Err(err) => last = Some(err.to_string()),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        panic!(
            "{} never reached {want:?} within {}s; it was last {}",
            self.handle.provider_id,
            within.as_secs(),
            last.unwrap_or_else(|| "never readable".into())
        );
    }
}

impl Drop for Spike {
    fn drop(&mut self) {
        let provider = self.provider.clone();
        let handle = self.handle.clone();
        let name = handle.provider_id.clone();
        // Blocking inside `Drop` because a destructor cannot await, and this
        // has to run even when the test is unwinding from a failed assertion:
        // that is exactly the run that would otherwise leave a machine behind.
        tokio::task::block_in_place(move || {
            tokio::runtime::Handle::current().block_on(async move {
                // Twice, a second apart. A test that failed mid-command leaves
                // an `exec` session the runtime is still winding down, and
                // `delete --force` refuses one of those with `clientIsStopped
                // … failed to delete process`. Every leftover from a failed run
                // was one of these: a 20 GiB volume and a network that the next
                // run then meets as its own debris.
                let Err(first) = provider.delete(&handle).await else {
                    return;
                };
                tokio::time::sleep(Duration::from_secs(1)).await;
                if let Err(again) = provider.delete(&handle).await {
                    eprintln!(
                        "could not release {name} ({first}), and again a second later ({again}); \
                         remove it with `container delete --force {name}` and \
                         `container volume delete {name}` and `container network delete {name}`"
                    );
                }
            });
        });
    }
}

/// The viewer proxy's resolver for exactly one machine.
///
/// The handle arrives after the proxy is listening, because the port the proxy
/// chose is what `Machine::vnc_url` builds its URL from: the machine cannot be
/// made until the port is known, and the port is not known until this exists.
struct OneMachine {
    provider: Arc<AppleContainer>,
    holds: std::sync::Mutex<Option<ProviderHandle>>,
}

#[async_trait]
impl ViewerResolver for OneMachine {
    async fn viewer_target(&self, computer: &str, port: u16) -> Option<ViewerTarget> {
        let handle = self.holds.lock().ok()?.clone()?;
        if handle.computer.to_string() != computer {
            return None;
        }
        self.provider.viewer_target(&handle, port).await.ok()
    }
}

/// 1. Create a computer from the pinned image.
///
/// One container, one volume, one network, all named for the computer, and the
/// name is what `list_owned` answers with.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn a_computer_is_made_as_a_container_a_volume_and_a_network() {
    let provider = runtime();
    let spike = Spike::make(&provider).await;
    let name = spike.handle.provider_id.clone();

    assert!(name.contains(&spike.computer.short()), "the name carries the computer's id: {name}");
    spike.settles_at(ProviderState::Running, SETTLE).await;

    for argv in [
        vec!["inspect", &name],
        vec!["volume", "inspect", &name],
        vec!["network", "inspect", &name],
    ] {
        let described = container(&argv).await;
        assert!(
            described.ok(),
            "`container {}` should describe what create made: {}",
            argv.join(" "),
            described.stderr
        );
    }

    // The one guess in `apple.rs` whose failure is silent and destructive. The
    // sweep matches this list against `provider_id` on the rows, so anything
    // other than the name here makes every live machine look unclaimed and the
    // first sweep after a restart deletes all of them.
    let owned = provider.list_owned().await.expect("this Mac's computers");
    assert!(
        owned.contains(&name),
        "list_owned must answer with container names; it answered {owned:?}. If these are \
         identifiers rather than names, `read_owned` reads the wrong field and the sweep will \
         delete live machines."
    );

    // The image's own watchdog is PID 1, and nothing was wrapped around it.
    // Read from the command line rather than from `comm`, which for a script
    // holds the interpreter's name on some kernels and the script's on others.
    let init = spike.run("tr '\\0' ' ' < /proc/1/cmdline").await;
    assert!(
        init.contains("guaca-init"),
        "PID 1 must be the image's watchdog: a supervisor around it would outlive the process \
         whose exit is how a machine sleeps. It is {init:?}"
    );
    // The file that watchdog measures, and that the app touches on every use.
    // Two different accounts touch it: PID 1 creates it as root, and the idle
    // ticker writes it through `exec` as uid 1000, which is why the image makes
    // it writable by both rather than by its owner.
    let beat = spike.run("stat -c '%Y %a' /run/guaca/heartbeat").await;
    assert!(beat.trim().split(' ').next().unwrap_or_default().parse::<u64>().is_ok(), "{beat:?}");
    spike.run("touch /run/guaca/heartbeat").await;

    // The home volume, handed to uid 1000 by that same root PID 1 on every
    // boot. This is the regression that a live run found: with `USER user` in
    // the image, PID 1 was itself uid 1000, a fresh volume's root belonged to
    // root, and nothing could write the home — the skeleton never arrived, XFCE
    // could not save its own config, and three tests failed on `Permission
    // denied` some way past the thing that caused it.
    let home = spike.run("stat -c '%u %g' /home/user").await;
    assert_eq!(
        home.trim(),
        "1000 1000",
        "the home volume must belong to the account commands run as, or an agent cannot write \
         its own home"
    );
    spike.run("touch /home/user/.guac-write-test && rm /home/user/.guac-write-test").await;
}

/// 2. Execute a command and preserve stdout, stderr, and exit code.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn a_command_keeps_its_two_streams_and_its_exit_code() {
    let provider = runtime();
    let spike = Spike::make(&provider).await;

    let out = provider
        .exec(
            &spike.handle,
            ExecRequest {
                argv: vec![
                    "/bin/bash".into(),
                    "-l".into(),
                    "-c".into(),
                    "echo on stdout; echo on stderr >&2; exit 3".into(),
                ],
                env: BTreeMap::new(),
                cwd: "/home/user".into(),
                timeout: Duration::from_secs(60),
            },
        )
        .await
        .expect("the command ran");

    // Kept apart, because the model is shown them apart: a runtime that merges
    // them turns every warning into part of an answer.
    assert_eq!(out.stdout.trim(), "on stdout", "stderr: {}", out.stderr);
    assert_eq!(out.stderr.trim(), "on stderr");
    assert_eq!(out.exit_code, 3, "the guest's exit code is the command's own");

    // The working directory and the account are what every command above the
    // boundary assumes, and a machine that runs them as root writes a browser
    // profile no agent can read.
    assert_eq!(spike.run("pwd").await.trim(), "/home/user");
    assert_eq!(spike.run("id -un").await.trim(), "user");
    assert_eq!(spike.run("id -u").await.trim(), "1000");
    // `-l` is a login shell, so `~/.profile` has been read and the shim
    // directory is ahead of the packaged browser.
    let path = spike.run("echo $PATH").await;
    assert!(path.contains("/home/user/.local/bin"), "the shim must be found first: {path}");
}

/// 3. Inject a sentinel for one command; prove it is absent from the next and
///    from provider inspection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn a_credential_reaches_one_command_and_nothing_else() {
    let provider = runtime();
    let spike = Spike::make(&provider).await;

    let carried = provider
        .exec(
            &spike.handle,
            ExecRequest {
                argv: vec![
                    "/bin/bash".into(),
                    "-l".into(),
                    "-c".into(),
                    "printenv GUAC_SPIKE_TOKEN".into(),
                ],
                env: BTreeMap::from([("GUAC_SPIKE_TOKEN".to_string(), SENTINEL.to_string())]),
                cwd: "/home/user".into(),
                timeout: Duration::from_secs(60),
            },
        )
        .await
        .expect("the command ran");
    assert_eq!(
        carried.stdout.trim(),
        SENTINEL,
        "`container exec --env NAME` must read the value from its own environment"
    );

    // The next command is a different process. A value that survives into it is
    // one the runtime wrote into the container, which is the disk this app
    // deliberately does not put a credential on.
    let after = spike.run("printenv GUAC_SPIKE_TOKEN || echo absent").await;
    assert_eq!(after.trim(), "absent", "a credential must not outlive the command it was for");

    // Nor into the container's own environment, which every later `exec` would
    // inherit if the runtime ever passed one along.
    //
    // Read defensively, and expect not to be able to read it at all: PID 1 is
    // root — the image leaves `USER` unset so that it can prepare the volume —
    // while commands arrive as uid 1000, and `/proc/1/environ` belongs to the
    // account that owns the process. A plain redirect would fail, count zero
    // matches, and pass this for the wrong reason, so the unreadable case is
    // told apart from the empty one.
    let inherited = spike
        .run(
            "if [ -r /proc/1/environ ]; then tr '\\0' '\\n' < /proc/1/environ | \
             grep -c '^GUAC_SPIKE_TOKEN=' || true; else echo unreadable; fi",
        )
        .await;
    assert!(
        matches!(inherited.trim(), "0" | "unreadable"),
        "the container's own environment holds the credential: {inherited}"
    );

    let described = container(&["inspect", &spike.handle.provider_id]).await;
    assert!(
        !described.stdout_str().contains(SENTINEL),
        "the value must not be readable from `container inspect`"
    );

    // The idle period is the one value this app does write into a container's
    // environment, and the watchdog is what reads it. It is not asserted from
    // in here — a uid-1000 session cannot read root's environment, so any check
    // from this side passes whether or not the setting arrived. The test that
    // proves it is the sleep one, which makes a machine with twenty seconds and
    // watches it stop.
}

/// 4. Place a binary attachment through the shared chunked `exec` path and read
///    it back byte for byte.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn a_binary_file_arrives_on_a_machine_byte_for_byte() {
    let provider = runtime();
    let spike = Spike::make(&provider).await;

    // Every byte value, including the ones a shell would eat, and big enough to
    // take several writes: the first truncates and the rest append, and a file
    // that fits in one command never exercises that.
    let bytes: Vec<u8> = (0..300 * 1024).map(|index| (index % 256) as u8).collect();
    let encoded = encode(&bytes);
    assert!(encoded.len() > PLACE_CHUNK, "the sample must take more than one write");

    let path = format!("{INBOX}/spike.bin");
    let mut first = true;
    for chunk in encoded.as_bytes().chunks(PLACE_CHUNK) {
        let chunk = String::from_utf8_lossy(chunk);
        let redirect = if first { ">" } else { ">>" };
        let command = place_command(&chunk, redirect, &path);
        // The live failure this closes: the whole write travels as one argv
        // string, Linux caps that at 128 KiB, and what comes back from over the
        // line is "failed to exec" — which names neither the file nor the
        // limit. Asserted here as well as in the unit test because this is the
        // one place a real kernel is on the other end.
        assert!(command.len() < MAX_GUEST_ARG, "one write is {} bytes", command.len());
        spike.run(&command).await;
        first = false;
    }

    let size = spike.run(&format!("stat -c %s {path}")).await;
    assert_eq!(size.trim(), bytes.len().to_string(), "the file arrived a different length");

    // Read back the same way a file leaves a machine: base64 on one line, and
    // the decoder that is paired with the encoder above.
    let read_back = decode_bytes(spike.run(&format!("base64 -w0 {path}")).await.trim());
    assert_eq!(read_back.len(), bytes.len());
    assert!(read_back == bytes, "the bytes that came back are not the bytes that went out");
}

/// 5. Start the desktop and load noVNC through Guaca's loopback proxy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn the_desktop_is_watchable_through_the_loopback_viewer() {
    let provider = runtime();

    // The proxy first: the port it chose is what the machine's URL is built
    // from, and the machine's handle is what the proxy resolves. One of the two
    // has to exist before the other, and the empty slot is how.
    let resolver = Arc::new(OneMachine { provider: provider.clone(), holds: Default::default() });
    let port = guac_lib::proxy::start(resolver.clone()).await.expect("a loopback viewer");

    let spike = Spike::with_idle(&provider, 900, port).await;
    *resolver.holds.lock().expect("the slot") = Some(spike.handle.clone());

    spike.machine.start_desktop().await.expect("the desktop starts");

    // Asked of the port rather than the process list: a process that exists is
    // not one that is serving, and this is the check that used to hand the
    // viewer a dead address and draw a black rectangle.
    let url = spike.machine.vnc_url().await.expect("noVNC is serving inside the guest");
    assert!(url.starts_with(&format!("http://127.0.0.1:{port}/")), "{url}");

    let page = reqwest::get(&url).await.expect("the viewer answered");
    assert_eq!(page.status(), 200, "the proxy could not reach the guest's noVNC");
    let body = page.text().await.expect("the page");
    assert!(
        body.contains("noVNC"),
        "the image must serve noVNC's own page at /opt/noVNC: {}",
        body.chars().take(300).collect::<String>()
    );

    // The page is not the desktop; the WebSocket is. noVNC opens it at the
    // page's host plus `/` + its `path` setting, so the upgrade is attempted
    // exactly where the URL tells it to look, through the proxy. A page that
    // loads over a socket that does not is the "Failed to connect to server"
    // banner an operator saw over a running desktop.
    let path = url
        .split("&path=")
        .nth(1)
        .and_then(|rest| rest.split('&').next())
        .expect("the viewer URL names its websocket path");
    let upgraded = websocket_upgrade(port, path).await;
    assert!(
        upgraded.starts_with("HTTP/1.1 101"),
        "the proxy must upgrade the socket at /{path}: {upgraded}"
    );
    assert!(upgraded.contains("RFB 003."), "and the bytes behind it are VNC's: {upgraded}");

    // The parts that had to be there for that to work, named individually so a
    // failure says which one is missing rather than "the desktop did not start".
    for check in [
        "pgrep -x Xvfb",
        "pgrep -x x11vnc",
        "pgrep -x xfce4-session",
        "test -x /opt/noVNC/utils/novnc_proxy",
        "test -f /opt/noVNC/vnc.html",
    ] {
        spike.run(check).await;
    }
    // Both directions of the bridge: noVNC is only useful because x11vnc is
    // behind it, and a guest with 5900 closed serves a page that never connects.
    spike.run("timeout 5 bash -c 'exec 3<>/dev/tcp/127.0.0.1/5900'").await;
}

/// 6. Launch Chromium, drive it over CDP through `browser.py`, take a
///    screenshot, and move the pointer with `xdotool`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn the_browser_is_driven_over_its_remote_interface_and_the_screen_photographed() {
    let provider = runtime();
    let spike = Spike::make(&provider).await;

    // `websocket-client` must already be there. E2B installs it on first
    // browse; a local machine may have no network at all, and a browse that
    // pauses to pip-install is a browse that fails on an aeroplane.
    spike.run("python3 -c 'import websocket'").await;

    // A page served from inside the guest rather than a `data:` URL, because
    // `browser.py` prefixes anything that is not http(s) with `https://`: a
    // `data:` URL handed to `open` becomes `https://data:…` and never loads.
    let page = "<!doctype html><title>Guaca spike</title><body>\
                <button id=go>Spike Button</button></body>";
    spike
        .run(&format!(
            "mkdir -p /tmp/guac-spike && echo {} | base64 -d > /tmp/guac-spike/index.html",
            encode(page.as_bytes())
        ))
        .await;
    spike
        .run(
            "(setsid python3 -m http.server 8099 --directory /tmp/guac-spike \
             >/tmp/guac-spike-server.log 2>&1 </dev/null &) ; sleep 2; \
             timeout 5 bash -c 'exec 3<>/dev/tcp/127.0.0.1/8099'",
        )
        .await;

    let read = spike
        .machine
        .browse("open", &serde_json::json!({ "url": "http://127.0.0.1:8099/" }))
        .await
        .expect("the browser opened the page");
    let described: serde_json::Value = serde_json::from_str(&read).expect("the driver's JSON");
    assert_eq!(described["title"], "Guaca spike", "the driver read a different page: {read}");
    assert!(
        read.contains("Spike Button"),
        "the driver must number the elements on the page it opened: {read}"
    );

    // Every browser on this machine, and which profile each one holds. Asked of
    // `/proc` rather than with `pgrep -f`, for the reason `desktop.rs`
    // documents at length: the pattern and the profile path both appear in the
    // command line of the shell doing the matching, so a `-f` search finds
    // itself and reports whatever it was asked to look for. `pgrep -x` matches
    // the executable name, which for the shell is `bash`.
    //
    // Two profiles on one machine is the failure the whole shim exists to
    // prevent, and it is invisible from inside the app: an operator signs in on
    // the screen, the session lands in a jar no agent drives, and detection
    // truthfully reports that the browser it reads is signed in to nothing.
    let profiles = spike
        .run(
            r#"for pid in $(pgrep -x chromium); do
                 args=$(tr '\0' ' ' < /proc/$pid/cmdline)
                 case "$args" in *--type=*) continue ;; esac
                 case "$args" in
                   *--user-data-dir=*)
                     printf '%s\n' "$args" | sed 's/.*--user-data-dir=\([^ ]*\).*/\1/' ;;
                   *) echo '(the default profile)' ;;
                 esac
               done | sort -u"#,
        )
        .await;
    let profiles: Vec<&str> =
        profiles.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    assert_eq!(
        profiles,
        ["/home/user/.guac/chrome"],
        "there must be exactly one browser profile in use on a machine, and it must be the one \
         agents drive"
    );

    let (shot, geometry) = spike.machine.screenshot().await.expect("a picture of the screen");
    assert!(shot.starts_with("data:image/jpeg;base64,"), "{}", &shot[..40.min(shot.len())]);
    assert_eq!(geometry.trim(), "1280x800", "the framebuffer is the size desktop.rs asks Xvfb for");
    let picture = decode_bytes(shot.trim_start_matches("data:image/jpeg;base64,"));
    assert!(
        picture.len() > 10_000,
        "a desktop with a browser on it is not {} bytes",
        picture.len()
    );

    // The pointer, which is the other half of what an agent can do to a screen.
    let moved = spike
        .machine
        .act_on_desktop(&DesktopAction::Move { x: 640, y: 400 })
        .await
        .expect("xdotool moved the pointer");
    assert_eq!(moved.exit_code, 0, "stderr: {}", moved.stderr);
    let where_it_is = spike.run("DISPLAY=:0 xdotool getmouselocation").await;
    assert!(
        where_it_is.contains("x:640"),
        "the pointer did not land where it was sent: {where_it_is}"
    );

    // Sign-in detection reads the profile from disk, so it has to answer on a
    // machine that has browsed. What it must not do is invent a session: a
    // browser that has seen nothing but its own loopback is signed in to
    // nothing, and a cookie here would be the false positive the signature
    // table exists to prevent.
    let state = spike.machine.signed_in_state().await.expect("the cookie jar was readable");
    assert!(
        state.cookies.is_empty(),
        "a machine that has only visited its own loopback holds no identity cookie: {:?}",
        state.cookies
    );
    println!("the browser reports having visited: {:?}", state.visited);
}

/// 7. Write a home file, stop, start, and verify the file and the Chrome
///    profile survive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn a_home_file_and_the_browser_profile_survive_a_sleep() {
    let provider = runtime();
    let spike = Spike::make(&provider).await;

    spike.run("echo 'work in progress' > /home/user/spike-home.txt").await;
    // Opening the browser is what creates the profile the volume exists to
    // keep, and what leaves the lock behind when the container stops.
    spike.machine.browse("read", &serde_json::json!({})).await.expect("the browser opened");
    spike.run("test -d /home/user/.guac/chrome/Default").await;

    provider.stop(&spike.handle).await.expect("the machine stops");
    spike.settles_at(ProviderState::Asleep, SETTLE).await;
    provider.start(&spike.handle, 900).await.expect("the machine wakes");
    spike.settles_at(ProviderState::Running, SETTLE).await;

    assert_eq!(
        spike.run("cat /home/user/spike-home.txt").await.trim(),
        "work in progress",
        "the volume is the whole reason a machine sleeps instead of being destroyed"
    );
    spike.run("test -d /home/user/.guac/chrome/Default").await;

    // A stopped container leaves Chrome's SingletonLock behind, and the next
    // Chrome refuses the profile as in use. PID 1 removes it on every boot, and
    // the proof is that the browser opens rather than that the file is gone.
    let lock = spike
        .run(
            "if [ -L /home/user/.guac/chrome/SingletonLock ] || \
             [ -e /home/user/.guac/chrome/SingletonLock ]; then echo present; else echo absent; fi",
        )
        .await;
    assert_eq!(lock.trim(), "absent", "PID 1 must clear the lock a stopped container left");
    spike.machine.browse("read", &serde_json::json!({})).await.expect("the profile is reusable");
}

/// 8. Let the heartbeat expire and verify the machine sleeps without losing its
///    disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn a_machine_nobody_is_using_stops_itself_and_keeps_its_disk() {
    let provider = runtime();
    // Twenty seconds, and the watchdog looks every thirty, so a machine nothing
    // touches is asleep within a minute. Nothing here touches the heartbeat:
    // that is the app's job, and this is the case where the app is gone.
    let spike = Spike::with_idle(&provider, 20, 0).await;

    spike.run("echo 'left behind' > /home/user/spike-idle.txt").await;
    spike.settles_at(ProviderState::Asleep, Duration::from_secs(240)).await;

    // Stopped, not deleted. The disk is what makes this sleep rather than loss,
    // and the volume outliving the container is the whole of that.
    let volume = container(&["volume", "inspect", &spike.handle.provider_id]).await;
    assert!(volume.ok(), "the home volume must outlive an idle stop: {}", volume.stderr);

    provider.start(&spike.handle, 20).await.expect("an idle machine can be woken");
    spike.settles_at(ProviderState::Running, SETTLE).await;
    assert_eq!(spike.run("cat /home/user/spike-idle.txt").await.trim(), "left behind");
}

/// 9. Destroy, and verify the container, the volume, the network and the viewer
///    target are all gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn destroying_a_computer_takes_all_three_of_its_resources() {
    let provider = runtime();
    let spike = Spike::make(&provider).await;
    let name = spike.handle.provider_id.clone();

    provider.delete(&spike.handle).await.expect("the machine is destroyed");

    assert_eq!(
        provider.inspect(&spike.handle).await.expect("the runtime answered"),
        ProviderState::Gone,
        "a destroyed machine must read as gone rather than as unreadable"
    );
    for argv in [
        vec!["inspect", &name],
        vec!["volume", "inspect", &name],
        vec!["network", "inspect", &name],
    ] {
        let described = container(&argv).await;
        assert!(
            !described.ok(),
            "`container {}` still describes something after destroy",
            argv.join(" ")
        );
    }

    let owned = provider.list_owned().await.expect("this Mac's computers");
    assert!(!owned.contains(&name), "a destroyed machine is still claimed: {owned:?}");

    // The pane asks for this when an operator opens a machine that has been
    // destroyed elsewhere, and the answer decides what they are told: `Gone` is
    // "make a new one", anything else is "try again".
    match provider.viewer_target(&spike.handle, VNC_PORT).await {
        Err(ProviderError::ResourceGone(said)) => assert!(said.contains(&name), "{said}"),
        other => panic!("a destroyed machine has nowhere to show a desktop, got {other:?}"),
    }

    // `Drop` deletes it again on the way out, which is the idempotency the
    // retry path depends on: a second delete must not be an error.
    drop(spike);
}

/// 10. The network boundary, with two agents.
///
/// One assertion and several measurements. The assertion is the release
/// blocker: an agent must not reach another agent's desktop. The rest are
/// recorded rather than asserted, because the spec documents host and LAN
/// reachability as a local-mode limitation rather than promising it is closed —
/// but it can only document it honestly if somebody measured it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Apple Container 1.2.x and the desktop image; run scripts/spike-apple.sh"]
async fn one_agent_cannot_reach_another_agents_desktop() {
    let provider = runtime();
    let first = Spike::make(&provider).await;
    let second = Spike::make(&provider).await;

    // A serving port, not a closed one. A refusal from a port with nothing
    // behind it would pass this test on a machine with no isolation at all.
    first.machine.start_desktop().await.expect("the first agent's desktop starts");
    let address = first.address().await;
    assert!(
        first.machine.vnc_url().await.is_some(),
        "the port must be serving before a refusal to reach it means anything"
    );

    let probe = |target: &str, port: u16| {
        format!(
            "timeout 8 bash -c 'exec 3<>/dev/tcp/{target}/{port}' 2>/dev/null && echo reachable \
             || echo refused"
        )
    };

    // The control: the machine can reach its own noVNC, so the address and the
    // port are right and the only variable left is which machine is asking.
    assert_eq!(
        first.run(&probe(&address, VNC_PORT)).await.trim(),
        "reachable",
        "the first agent cannot reach its own desktop, so this test proves nothing"
    );
    assert_eq!(
        second.run(&probe(&address, VNC_PORT)).await.trim(),
        "refused",
        "one agent reached another agent's desktop at {address}:{VNC_PORT}. This is a release \
         blocker: every local agent must be on its own network."
    );

    // Everything below is printed for `docs/LOCAL_COMPUTERS.md` under "Spike
    // results". The spike script repeats the list; these are the guest's own
    // answers, which is what the document should record.
    println!("--- network measurements, from inside an agent's machine ---");
    // `/usr/sbin` is not on an unprivileged account's PATH on Debian, and `ip`
    // lives there: without this the measurement is a "command not found" that
    // reads as a machine with no gateway.
    let gateway =
        second.run("PATH=$PATH:/usr/sbin ip route | awk '/^default/ {print $3; exit}'").await;
    let gateway = gateway.trim().to_string();
    println!("gateway (the Mac, from the guest): {gateway}");
    println!("guest address of the other agent:  {address}");

    let mut measurements = vec![
        (
            "public DNS resolution".to_string(),
            "getent hosts example.com >/dev/null && echo reachable || echo refused".to_string(),
        ),
        ("public HTTP (example.com:80)".to_string(), probe("example.com", 80)),
        ("public HTTPS (example.com:443)".to_string(), probe("example.com", 443)),
        ("arbitrary TCP (1.1.1.1:53)".to_string(), probe("1.1.1.1", 53)),
    ];
    if !gateway.is_empty() {
        // Whatever the operator bound to Mac loopback before running the spike:
        // the script tells them to, because a port with nothing on it is
        // indistinguishable from a port that is blocked.
        let port: u16 = std::env::var("GUAC_SPIKE_HOST_PORT")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(8765);
        measurements.push((format!("the Mac at {gateway}:{port}"), probe(&gateway, port)));
    }
    match std::env::var("GUAC_SPIKE_LAN") {
        Ok(lan) if !lan.trim().is_empty() => {
            measurements.push((format!("a LAN address ({lan}:80)"), probe(lan.trim(), 80)));
        }
        _ => println!(
            "a LAN address: not measured. Set GUAC_SPIKE_LAN=<address on your network> and run \
             this again."
        ),
    }
    for (what, command) in measurements {
        println!("{what}: {}", second.run(&command).await.trim());
    }
}

/// One WebSocket upgrade through the loopback viewer at `/{path}`, answered
/// with the response head and whatever the server sends first. Written on a
/// raw socket because reqwest cannot upgrade and the proxy is a byte relay.
async fn websocket_upgrade(viewer_port: u16, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", viewer_port))
        .await
        .expect("the viewer is listening");
    let head = format!(
        "GET /{path} HTTP/1.1\r\nHost: 127.0.0.1:{viewer_port}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: binary\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).await.expect("the request was written");
    let mut buf = vec![0u8; 4096];
    let mut got = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && !String::from_utf8_lossy(&got).contains("RFB") {
        match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => got.extend_from_slice(&buf[..n]),
            Ok(Err(_)) => break,
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

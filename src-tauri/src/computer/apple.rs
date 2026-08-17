//! Apple Container: an agent's computer as a Linux VM on the operator's Mac.
//!
//! Every operation here is an argument vector handed to `cli.rs`, which spawns
//! the signed `container` binary directly. Nothing builds a shell command, and
//! a credential is never an argument: `exec` passes `--env NAME` and the value
//! travels in the child's own environment, which is the one thing in this file
//! that would be a release blocker to get wrong.
//!
//! One agent gets one network, one volume and one container, all named
//! `guac-<computer>`. Ownership is the labels rather than the name: a second
//! copy of Guac on the same Mac makes resources that look identical, and the
//! sweep deletes what it believes it owns.
//!
//! Apple Container is not installed on the machine this was written on, so
//! every claim about what it prints comes from Apple's 1.2.2 documentation and
//! sources. The guesses are marked; the spike confirms them against the real
//! binary before anyone relies on them.

use std::collections::BTreeMap;
use std::time::Duration;

use super::cli::{Cli, CliError, CliOutput};
use super::image;
use super::provider::{
    timed_out, ComputerProvider, CreateComputer, ExecRequest, Output, ProviderError,
    ProviderHandle, ProviderReadiness, ProviderState, ProviderStatus, ViewerTarget,
};
use crate::domain::computer::{Provider, Secret};
use crate::domain::ids::ComputerId;

/// Where the signed package puts it. Looked at before `PATH` because a Mac app
/// launched from Finder inherits neither Homebrew's directory nor this one.
const WELL_KNOWN: &[&str] = &["/usr/local/bin/container"];

/// The first release this was tested against, and the first major nothing here
/// promises anything about.
pub const MIN_VERSION: (u32, u32, u32) = (1, 2, 2);
const UNSUPPORTED_MAJOR: u32 = 2;

/// Whether this build could drive Apple Container at all. The operating system
/// is the whole of the compile-time question, and deliberately so: the
/// architecture of *this* binary says nothing about the one it spawns, because
/// macOS runs a native `container` natively even when the process asking for it
/// is translated. Gating on `target_arch` reported "unsupported" on the very
/// Macs this is built and tested on. What the machine can actually do is
/// settled by whether the binary is there at all — it installs on nothing but
/// macOS 26 on Apple silicon — and then by the version window.
const SUPPORTED_PLATFORM: bool = cfg!(target_os = "macos");

/// The guest's home: the volume's mount point, and where commands run. The
/// image puts the unprivileged account here and the desktop code assumes it.
const GUEST_HOME: &str = "/home/user";

/// Long enough for a runtime that is busy with another container, short enough
/// that an operator's click is not held indefinitely.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(60);

/// A first pull is a desktop image over whatever connection the Mac has.
const PULL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Starting the service installs a kernel on a first run.
const SERVICE_TIMEOUT: Duration = Duration::from_secs(120);

/// Taken away from every agent's container. Guest root may still install
/// packages; it cannot get back a capability outside the bounding set.
const DROPPED_CAPABILITIES: &[&str] =
    &["NET_ADMIN", "NET_RAW", "SYS_ADMIN", "SYS_MODULE", "SYS_PTRACE"];

const CPUS: &str = "4";
const MEMORY: &str = "4G";
const SHM_SIZE: &str = "1G";
/// The quota Apple enforces when the volume is made, which is the only moment
/// it can be enforced.
const HOME_SIZE: &str = "20G";

/// What to tell an operator whose Mac has no runtime on it. Names where the
/// signed package is, because Apple Container is in no package manager.
///
/// This is where the hardware is spoken about, rather than at compile time,
/// because this is the point where it is actually known: the package installs
/// on nothing but macOS 26 on Apple silicon, so a Mac with no `container` on it
/// is either one of those without the download or a Mac that will never have
/// one, and the same sentence serves both.
const INSTALL_HINT: &str =
    "Apple Container needs macOS 26 on Apple silicon; this Mac has no `container` binary at \
     /usr/local/bin/container or on PATH. Install the signed package from \
     github.com/apple/container/releases, then start a computer.";

/// Apple Container, behind the boundary.
///
/// Holds no credential and no per-agent state: everything it needs to reach a
/// machine is the name on the handle, and everything an agent's command needs
/// arrives on the request.
pub struct AppleContainer {
    cli: Cli,
    /// Scopes every label this makes, so another copy of Guac on the same Mac
    /// is never swept up as this one's orphan.
    installation: String,
    image: String,
    /// Whether this build could drive the runtime at all: `SUPPORTED_PLATFORM`
    /// for anything `discover` made. Held rather than read from the constant at
    /// each site so that both answers are testable wherever the suite runs,
    /// including on a machine that is not a Mac.
    platform: bool,
}

impl AppleContainer {
    /// `None` when the binary is not on this Mac, which is how the manager
    /// tells "not installed" from "installed and refusing".
    pub fn discover(installation: &str) -> Option<AppleContainer> {
        Cli::discover("container", WELL_KNOWN).map(|cli| {
            AppleContainer::with_cli(cli, installation, image::image_ref(), SUPPORTED_PLATFORM)
        })
    }

    pub fn with_cli(cli: Cli, installation: &str, image: String, platform: bool) -> AppleContainer {
        AppleContainer { cli, installation: installation.to_string(), image, platform }
    }

    /// Starts the service if it is not running.
    ///
    /// Called when somebody asked for a machine, never on a probe: starting a
    /// virtualisation service is a thing an operator should be able to predict
    /// from what they clicked.
    ///
    /// The gate is here rather than in `create` alone because this is the door
    /// every made machine goes through, and a provider that leans on the
    /// manager having consulted `probe` first is one refusal away from starting
    /// a service this build cannot then drive.
    pub async fn ensure_running(&self) -> Result<(), ProviderError> {
        self.require_supported().await?;
        if self.control(&status_argv()).await?.ok() {
            return Ok(());
        }
        let started =
            self.cli.run(&start_service_argv(), &BTreeMap::new(), SERVICE_TIMEOUT).await?;
        if !started.ok() {
            return Err(ProviderError::Unavailable(format!(
                "Apple Container's service would not start: {}; run `container system start` in \
                 Terminal to see what it says",
                detail(&started)
            )));
        }
        Ok(())
    }

    /// One control-plane command, with no credentials: nothing this provider
    /// does apart from `exec` has any business holding one.
    async fn control(&self, argv: &[String]) -> Result<CliOutput, ProviderError> {
        Ok(self.cli.run(argv, &BTreeMap::new(), CONTROL_TIMEOUT).await?)
    }

    /// The three-CLI-calls half of `probe`, apart from the platform question so
    /// that it can be asked on any machine.
    async fn probe_runtime(&self) -> ProviderStatus {
        if let Err(reason) = self.version_or_reason().await {
            return reason;
        }
        // Both of the last two offer to start it, because starting a computer
        // is what starts the service either way. They are said differently
        // because "it says it is stopped" and "it did not answer at all" are
        // different things to walk into.
        match self.control(&status_argv()).await {
            Ok(answer) if answer.ok() => {
                ProviderStatus::ready("Apple Container is installed and running.")
            }
            Ok(_) => service_stopped(),
            Err(_) => service_silent(),
        }
    }

    /// The version the runtime reports, or why this build will not drive it.
    ///
    /// One answer with two readers: Settings draws it as a status and
    /// `require_supported` refuses with it, so an operator meets the same
    /// sentence whichever of the two showed it to them.
    async fn version_or_reason(&self) -> Result<(u32, u32, u32), ProviderStatus> {
        let spoke = match self.cli.run(&version_argv(), &BTreeMap::new(), CONTROL_TIMEOUT).await {
            Ok(spoke) => spoke,
            // Nothing to run. Every other failure means it is installed and
            // something else is wrong, and saying "not installed" would send
            // the operator to install what they already have.
            Err(CliError::Spawn(_)) => return Err(not_installed()),
            Err(err) => return Err(version_unanswered(&err)),
        };
        let Some(found) = parse_version(&format!("{}\n{}", spoke.stdout_str(), spoke.stderr))
        else {
            return Err(version_unreadable(&spoke));
        };
        if !supported(found) {
            return Err(version_out_of_range(found));
        }
        Ok(found)
    }

    /// Refuses, in the words `probe` would have used, when this build cannot
    /// drive what is installed.
    ///
    /// Asked where a machine is *made*, and deliberately not where an existing
    /// one is used: an operator who upgraded the runtime past this build's
    /// range still has machines with a browser signed in and work on the disk,
    /// and refusing to talk to those would strand them.
    async fn require_supported(&self) -> Result<(), ProviderError> {
        if !self.platform {
            return Err(refusal(unsupported_platform()));
        }
        self.version_or_reason().await.map(|_| ()).map_err(refusal)
    }

    /// Pulls the image if the runtime does not already have it. A machine that
    /// exists with no image to boot is worse than one that was never made.
    async fn ensure_image(&self) -> Result<(), ProviderError> {
        if self.control(&image_present_argv(&self.image)).await?.ok() {
            return Ok(());
        }
        // Its own deadline, and a long one: a first pull is a desktop image
        // over whatever connection the Mac has, and the control-plane timeout
        // would abandon it a minute in and leave the operator to start again.
        let pulled = self.cli.run(&pull_argv(&self.image), &BTreeMap::new(), PULL_TIMEOUT).await?;
        if !pulled.ok() {
            return Err(ProviderError::Image(format!(
                "the desktop image could not be pulled: {}; check your network, or set {} to a \
                 locally built image",
                detail(&pulled),
                image::IMAGE_ENV
            )));
        }
        Ok(())
    }

    /// Why one of the two shared-name resources refused to be created, which
    /// decides whether this create can carry on with what is already there.
    ///
    /// Fails closed at every step: a runtime that will not describe the thing,
    /// output this build cannot read, and labels that are absent are all
    /// somebody else's, because the alternative is mounting a stranger's home
    /// volume into an agent's machine and deleting it on the way out.
    async fn why_refused(
        &self,
        refused: &CliOutput,
        inspect: &[String],
        computer: ComputerId,
    ) -> Result<Refusal, ProviderError> {
        if !already_exists(refused) {
            return Ok(Refusal::Other);
        }
        let described = self.control(inspect).await?;
        if !described.ok() {
            return Ok(Refusal::SomebodyElses);
        }
        let Ok(described) = described.json() else {
            return Ok(Refusal::SomebodyElses);
        };
        Ok(if left_by(&described, computer) {
            Refusal::OurLeftover
        } else {
            Refusal::SomebodyElses
        })
    }

    /// Makes the network, the volume and the container, recording each as it
    /// succeeds so a failure knows what there is to unmake.
    async fn assemble(
        &self,
        request: &CreateComputer,
        made: &mut Vec<Made>,
    ) -> Result<(), ProviderError> {
        let name = resource_name(request.computer);

        // A name already taken is usually this computer's own leftover, from a
        // create that failed and could not tidy up after itself, and taking it
        // over is the only way that computer is ever made again. Usually, not
        // certainly: the name carries eight hex characters of a random id, so
        // another installation holding it is unlikely rather than impossible,
        // and what would be adopted then is a stranger's home volume — mounted
        // into this agent's machine, and deleted along with it. So the label is
        // asked for, and anything that cannot be shown to be ours is refused.
        // Adopted or made, it goes in `made`, so a failure later in this call
        // unmakes it either way.
        let network =
            self.control(&network_create_argv(&self.installation, request.computer)).await?;
        if !network.ok() {
            match self.why_refused(&network, &network_inspect_argv(&name), request.computer).await?
            {
                Refusal::OurLeftover => {}
                Refusal::SomebodyElses => {
                    return Err(name_taken("private network", &name, "container network delete"))
                }
                Refusal::Other => {
                    return Err(step_failed("creating the computer's private network", &network))
                }
            }
        }
        made.push(Made::Network);

        let volume =
            self.control(&volume_create_argv(&self.installation, request.computer)).await?;
        if !volume.ok() {
            match self.why_refused(&volume, &volume_inspect_argv(&name), request.computer).await? {
                Refusal::OurLeftover => {}
                Refusal::SomebodyElses => {
                    return Err(name_taken("home volume", &name, "container volume delete"))
                }
                Refusal::Other => {
                    return Err(step_failed("creating the computer's home volume", &volume))
                }
            }
        }
        made.push(Made::Volume);

        let created =
            self.control(&container_create_argv(request, &self.installation, &self.image)).await?;
        if !created.ok() {
            return Err(step_failed("creating the computer", &created));
        }
        made.push(Made::Container);

        let started = self.control(&start_argv(&name)).await?;
        if !started.ok() {
            return Err(step_failed("starting the computer", &started));
        }
        Ok(())
    }

    /// Best-effort teardown of a half-made computer, newest first.
    ///
    /// Only what this call made or adopted — and adoption is only ever of a
    /// resource that carries this computer's own label, so everything in `made`
    /// is this computer's either way. A forced delete of a container this call
    /// never created would destroy a machine that happens to share a name. A
    /// failure here is logged rather than returned, because the caller is
    /// already reporting why the computer could not be made and that is the
    /// more useful of the two.
    async fn unmake(&self, name: &str, made: &[Made]) {
        for kind in made.iter().rev() {
            let argv = match kind {
                Made::Container => delete_argv(name),
                Made::Volume => volume_delete_argv(name),
                Made::Network => network_delete_argv(name),
            };
            match self.control(&argv).await {
                Ok(out) if out.ok() || missing(&out) => {}
                Ok(out) => tracing::warn!(
                    computer = %name,
                    ?kind,
                    said = %detail(&out),
                    "could not release part of a computer that was never finished"
                ),
                Err(err) => tracing::warn!(
                    %err,
                    computer = %name,
                    ?kind,
                    "could not release part of a computer that was never finished"
                ),
            }
        }
    }
}

/// What a create has actually made, so a failure unmakes that and nothing
/// else: force-deleting a container this call did not create would destroy a
/// machine that only shares a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Made {
    Network,
    Volume,
    Container,
}

/// What a refused `network create` or `volume create` turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The name is taken by something carrying this computer's own label: a
    /// create that got this far and could not tidy up after itself.
    OurLeftover,
    /// The name is taken by something that is not demonstrably ours.
    SomebodyElses,
    /// It refused for some other reason entirely.
    Other,
}

#[async_trait::async_trait]
impl ComputerProvider for AppleContainer {
    fn kind(&self) -> Provider {
        Provider::AppleContainer
    }

    async fn probe(&self) -> ProviderStatus {
        // Asked before anything is spawned, because on something that is not a
        // Mac there is nothing to look for and nothing worth a process.
        if !self.platform {
            return unsupported_platform();
        }
        self.probe_runtime().await
    }

    /// One network, one volume, one container, started.
    ///
    /// Anything that fails takes back exactly what this call made: a network
    /// and a volume left behind are invisible from inside the app and hold the
    /// home quota each, and the operator is looking at an error that says the
    /// computer was not made.
    async fn create(&self, request: &CreateComputer) -> Result<ProviderHandle, ProviderError> {
        self.ensure_running().await?;
        self.ensure_image().await?;

        let name = resource_name(request.computer);
        let mut made = Vec::new();
        if let Err(err) = self.assemble(request, &mut made).await {
            self.unmake(&name, &made).await;
            return Err(err);
        }

        // No secrets: a machine on this Mac is reached by name over a socket
        // only this user can open, so there is no token to issue and none to
        // store.
        Ok(ProviderHandle {
            computer: request.computer,
            provider_id: name,
            control_secret: Secret::default(),
            viewer_secret: Secret::default(),
        })
    }

    async fn inspect(&self, handle: &ProviderHandle) -> Result<ProviderState, ProviderError> {
        let name = &handle.provider_id;
        let described = self.control(&inspect_argv(name)).await?;
        if !described.ok() {
            // The one failure that is an answer. Everything else is a runtime
            // that would not say, and a machine nobody could ask about is not
            // a machine that is gone.
            if missing(&described) {
                return Ok(ProviderState::Gone);
            }
            return Err(step_failed("looking up the computer", &described));
        }
        read_state(name, &described.json()?)
    }

    /// Wakes it under the name it already had: nothing about a local machine is
    /// reissued, so the handle the manager holds stays correct.
    async fn start(
        &self,
        handle: &ProviderHandle,
        _idle_seconds: u32,
    ) -> Result<ProviderHandle, ProviderError> {
        let started = self.control(&start_argv(&handle.provider_id)).await?;
        if !started.ok() {
            return Err(step_failed("starting the computer", &started));
        }
        Ok(handle.clone())
    }

    /// Nothing to say to the runtime: what makes a local machine sleep is the
    /// manager's ticker and the watchdog in the image, and neither of them
    /// takes an instruction from here.
    async fn keep_awake(&self, _handle: &ProviderHandle, _idle_seconds: u32) {}

    /// Stops the container, keeping its volume. The volume is the point: a
    /// browser that was signed in still is when it wakes.
    async fn stop(&self, handle: &ProviderHandle) -> Result<(), ProviderError> {
        let stopped = self.control(&stop_argv(&handle.provider_id)).await?;
        if !stopped.ok() {
            return Err(step_failed("stopping the computer", &stopped));
        }
        Ok(())
    }

    /// Container, volume, network, in that order, tolerating what is already
    /// gone: this is retried after a failure, and the second attempt finds most
    /// of it missing. A real failure stops the sequence and is reported, so the
    /// row stays and the retry happens rather than the rest leaking silently.
    async fn delete(&self, handle: &ProviderHandle) -> Result<(), ProviderError> {
        let name = &handle.provider_id;
        let removals = [
            ("removing the computer", delete_argv(name)),
            ("removing the computer's home volume", volume_delete_argv(name)),
            ("removing the computer's private network", network_delete_argv(name)),
        ];
        for (step, argv) in removals {
            let removed = self.control(&argv).await?;
            if !removed.ok() && !missing(&removed) {
                return Err(step_failed(step, &removed));
            }
        }
        Ok(())
    }

    /// One command inside the machine, with whatever credentials its group
    /// holds. The values go to `Cli::run`, which puts them in the child's
    /// environment; the vector it is given names them and nothing more.
    async fn exec(
        &self,
        handle: &ProviderHandle,
        request: ExecRequest,
    ) -> Result<Output, ProviderError> {
        let argv = exec_argv(&handle.provider_id, &request);
        let ran = match self.cli.run(&argv, &request.env, request.timeout).await {
            Ok(ran) => ran,
            // Said in the model's terms rather than the runtime's: the process
            // is still running in there, and the way past a deadline is to stop
            // waiting on it.
            Err(CliError::Timeout { .. }) => return Err(timed_out(request.timeout)),
            Err(err) => return Err(err.into()),
        };
        Ok(Output {
            stdout: String::from_utf8_lossy(&ran.stdout).into_owned(),
            stderr: ran.stderr,
            exit_code: ran.status,
        })
    }

    /// Straight at the guest's address on its own network, which this Mac can
    /// route to and nothing else can. Nothing is published, so there is no
    /// token to carry and no TLS to terminate.
    async fn viewer_target(
        &self,
        handle: &ProviderHandle,
        port: u16,
    ) -> Result<ViewerTarget, ProviderError> {
        let name = &handle.provider_id;
        let described = self.control(&inspect_argv(name)).await?;
        if !described.ok() {
            if missing(&described) {
                return Err(ProviderError::ResourceGone(format!(
                    "{name} is no longer on this Mac, so its desktop cannot be shown; destroy the \
                     computer from its pane and make a new one"
                )));
            }
            return Err(step_failed("looking up the computer's address", &described));
        }
        Ok(ViewerTarget {
            tls: false,
            host: read_address(name, &described.json()?)?,
            port,
            headers: vec![],
        })
    }

    async fn list_owned(&self) -> Result<Vec<String>, ProviderError> {
        let listed = self.control(&list_argv()).await?;
        if !listed.ok() {
            return Err(step_failed("listing this Mac's computers", &listed));
        }
        Ok(read_owned(&listed.json()?, &self.installation))
    }
}

/// The name the network, the volume and the container all share. One string
/// for three kinds because the CLI keeps them in separate namespaces, and a
/// person reading `container ls` should see which volume belongs to which
/// machine without a lookup.
fn resource_name(computer: ComputerId) -> String {
    format!("guac-{}", computer.short())
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

fn version_argv() -> Vec<String> {
    argv(&["--version"])
}

fn status_argv() -> Vec<String> {
    argv(&["system", "status"])
}

/// The flag is not optional. Without it a first run asks the operator for
/// permission to install a kernel, and a child spawned with no terminal waits
/// on that prompt until its deadline.
fn start_service_argv() -> Vec<String> {
    argv(&["system", "start", "--enable-kernel-install"])
}

fn image_present_argv(image: &str) -> Vec<String> {
    argv(&["image", "inspect", image])
}

fn pull_argv(image: &str) -> Vec<String> {
    argv(&["image", "pull", image])
}

fn network_create_argv(installation: &str, computer: ComputerId) -> Vec<String> {
    let mut parts = argv(&["network", "create"]);
    parts.extend(owner_labels(installation, computer));
    parts.push(resource_name(computer));
    parts
}

fn network_delete_argv(name: &str) -> Vec<String> {
    argv(&["network", "delete", name])
}

/// Asked only of a name that is already taken, to find out whose it is.
fn network_inspect_argv(name: &str) -> Vec<String> {
    argv(&["network", "inspect", name])
}

/// The quota is given here because here is where it can be given: a volume
/// that was made without one cannot be held to it later, and the agent's home
/// is the only place a runaway download can land.
fn volume_create_argv(installation: &str, computer: ComputerId) -> Vec<String> {
    let mut parts = argv(&["volume", "create", "-s", HOME_SIZE]);
    parts.extend(owner_labels(installation, computer));
    parts.push(resource_name(computer));
    parts
}

fn volume_delete_argv(name: &str) -> Vec<String> {
    argv(&["volume", "delete", name])
}

fn volume_inspect_argv(name: &str) -> Vec<String> {
    argv(&["volume", "inspect", name])
}

fn container_create_argv(request: &CreateComputer, installation: &str, image: &str) -> Vec<String> {
    let name = resource_name(request.computer);
    let mut parts = argv(&["create", "--name", &name]);
    // Its own network, never the shared default one: agents on one network can
    // reach each other's desktops, and nothing about that is asked for.
    parts.extend(argv(&["--network", &name]));
    // The only mount there is. A named volume rather than a bind mount, so
    // nothing of this Mac's filesystem is inside an agent's machine.
    parts.extend(argv(&[
        "--mount",
        &format!("type=volume,source={name},target={GUEST_HOME}"),
        "--cpus",
        CPUS,
        "--memory",
        MEMORY,
        "--shm-size",
        SHM_SIZE,
    ]));
    for capability in DROPPED_CAPABILITIES {
        parts.extend(argv(&["--cap-drop", capability]));
    }
    parts.extend(owner_labels(installation, request.computer));
    // Diagnostic, for a person reading `container ls`. Never the ownership
    // key: agents are renamed and deleted, and their machines outlive both.
    parts.extend(argv(&["--label", &format!("guac.agent={}", request.agent)]));
    // The one `--env NAME=value` in this file. It is a number the operator
    // chose, read by the watchdog in the image to decide when the machine has
    // been idle long enough to stop itself.
    parts.extend(argv(&["--env", &format!("GUAC_IDLE_SECONDS={}", request.idle_seconds)]));
    parts.push(image.to_string());
    parts
}

fn start_argv(name: &str) -> Vec<String> {
    argv(&["start", name])
}

/// Ten seconds before the runtime insists, which is the desktop's chance to
/// write out the browser profile the volume exists to keep.
fn stop_argv(name: &str) -> Vec<String> {
    argv(&["stop", "--time", "10", name])
}

/// Always forced. A running container refuses to be deleted, and every caller
/// here is removing a machine on purpose — including the rollback of a create
/// that got as far as starting one.
fn delete_argv(name: &str) -> Vec<String> {
    argv(&["delete", "--force", name])
}

fn inspect_argv(name: &str) -> Vec<String> {
    argv(&["inspect", name])
}

/// `--all`, because a stopped machine is one of this app's just as much as a
/// running one, and it is the stopped orphan that sits on 20 GiB unnoticed.
fn list_argv() -> Vec<String> {
    argv(&["ls", "--all", "--format", "json"])
}

/// One command inside a machine.
///
/// The variables are named and never given: `container exec --env NAME` reads
/// the value from its own environment, which `Cli::run` fills from the map
/// that never becomes an argument. Sorted because the map is, so the vector a
/// test pins is the vector every run produces.
fn exec_argv(name: &str, request: &ExecRequest) -> Vec<String> {
    let mut parts = argv(&["exec", "--workdir", &request.cwd]);
    for variable in request.env.keys() {
        parts.extend(argv(&["--env", variable]));
    }
    parts.push(name.to_string());
    parts.extend(request.argv.iter().cloned());
    parts
}

/// The three labels that make a resource this installation's.
///
/// Ownership is these rather than the name: a second copy of Guac on the same
/// Mac makes resources named identically, and the sweep deletes what it
/// believes it owns.
fn owner_labels(installation: &str, computer: ComputerId) -> Vec<String> {
    argv(&[
        "--label",
        "guac=true",
        "--label",
        &format!("guac.installation={installation}"),
        "--label",
        &format!("guac.computer={computer}"),
    ])
}

/// What one container is doing, from the array `container inspect` prints.
///
/// Anything outside the known words is an error rather than `Gone`, because
/// `Gone` is permission to throw a disk away: an unfamiliar word is far more
/// likely a machine that is fine and a build that is old. A reply that is not
/// an array at all is the same kind of unknown, and must not read as an empty
/// one.
fn read_state(name: &str, inspect: &serde_json::Value) -> Result<ProviderState, ProviderError> {
    let Some(described) = inspect.as_array() else {
        return Err(ProviderError::Operation(format!(
            "Apple Container described {name} in a form this build does not understand; try \
             again, and if it persists destroy the computer from its pane"
        )));
    };
    // Nothing in the array is the runtime positively saying there is no such
    // container, which is the one answer that permits replacing a disk.
    let Some(container) = described.first() else {
        return Ok(ProviderState::Gone);
    };

    match container["status"].as_str().unwrap_or_default() {
        "running" => Ok(ProviderState::Running),
        // All three keep the volume, which is what sleeping is for. `created`
        // is one that was made and never started; the manager wakes it exactly
        // as it wakes a stopped one.
        "stopped" | "created" | "exited" => Ok(ProviderState::Asleep),
        other => Err(ProviderError::Operation(format!(
            "Apple Container reports {name} in state {other:?}, which this build does not \
             understand; try again, and if it persists destroy the computer from its pane"
        ))),
    }
}

/// Where the viewer proxy connects, from the same array.
fn read_address(name: &str, inspect: &serde_json::Value) -> Result<String, ProviderError> {
    let Some(container) = inspect.as_array().and_then(|described| described.first()) else {
        return Err(ProviderError::ResourceGone(format!(
            "{name} is no longer on this Mac, so there is nowhere to show its desktop; destroy \
             the computer from its pane and make a new one"
        )));
    };
    let Some(address) = container["networks"][0]["address"].as_str() else {
        return Err(ProviderError::Operation(format!(
            "{name} has no address on its private network yet; wait a moment and open the pane \
             again, and if it persists stop and start the computer"
        )));
    };
    // `192.168.64.3/24`: the prefix length describes the network, and what the
    // proxy needs is the host.
    Ok(address.split('/').next().unwrap_or(address).to_string())
}

/// Every container on this Mac that this installation made.
///
/// What this returns is what the sweep deletes, so the labels have to be there
/// and have to be ours: a container carrying no label at all belongs to
/// something else, and one carrying another installation's belongs to another
/// copy of Guac whose agents are still using it.
///
/// **`configuration.id` must be the container's name.** The sweep matches what
/// this returns against `provider_id` on the rows, and `provider_id` is the
/// name `create` chose. If Apple puts anything else there — a content digest,
/// a generated identifier — every live machine looks unclaimed and the first
/// sweep after a restart deletes all of them. It is the one guess in this file
/// whose failure is silent and destructive, so the spike confirms it before
/// Task 4 lets the sweep act on it.
fn read_owned(list: &serde_json::Value, installation: &str) -> Vec<String> {
    list.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|listed| ours(listed, installation))
        .filter_map(|listed| listed["configuration"]["id"].as_str().map(str::to_string))
        .collect()
}

/// Whether an inspected resource is one this very computer left behind.
///
/// The name is not evidence: it carries eight hex characters of a random id,
/// so another installation holding the same one is unlikely rather than
/// impossible, and what would be adopted on the strength of a name alone is a
/// stranger's home volume. The label is the evidence, read down the same path
/// as `read_owned` uses and just as defensively — absent, unreadable, or
/// anyone else's all mean not ours.
///
/// The array is unwrapped if there is one, because whether `network inspect`
/// answers with an array or a bare object is unconfirmed until the spike.
/// Being relaxed about that shape is safe while being strict about the label:
/// the worst a misread shape does is refuse an adoption that would have been
/// allowed.
fn left_by(described: &serde_json::Value, computer: ComputerId) -> bool {
    let described = described.as_array().and_then(|items| items.first()).unwrap_or(described);
    let owner = computer.to_string();
    described["configuration"]["labels"]["guac.computer"].as_str() == Some(owner.as_str())
}

/// A name held by something this computer cannot show it owns.
///
/// The remedy is manual and says so: this build will not delete a resource it
/// cannot prove is its own, and the operator is the only one who can tell
/// whether the thing in the way is theirs to remove.
fn name_taken(kind: &str, name: &str, delete_command: &str) -> ProviderError {
    ProviderError::Operation(format!(
        "the computer's {kind} could not be made: something on this Mac is already called {name}, \
         and it does not carry this computer's label. Remove it with `{delete_command} {name}` if \
         it is yours to remove, then try again."
    ))
}

/// Whether one listed container carries this installation's ownership labels.
///
/// The path to them is a guess from Apple's 1.2.2 how-to and the spike
/// confirms it; until then a shape this cannot read owns nothing, which loses
/// an orphan rather than deleting a stranger.
fn ours(listed: &serde_json::Value, installation: &str) -> bool {
    let labels = &listed["configuration"]["labels"];
    labels["guac"].as_str() == Some("true")
        && labels["guac.installation"].as_str() == Some(installation)
}

/// The first three dotted numbers in whatever `container --version` printed.
///
/// Deliberately loose: the exact wording is unconfirmed until the spike, and a
/// parser pinned to a sentence Apple then reworded would report every install
/// as unreadable.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    text.split(|c: char| !c.is_ascii_digit() && c != '.').find_map(|token| {
        let mut numbers = token.split('.').map(str::parse::<u32>);
        match (numbers.next(), numbers.next(), numbers.next()) {
            (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch))) => Some((major, minor, patch)),
            _ => None,
        }
    })
}

/// Whether this build will drive that release. Both ends matter: below the
/// floor the flags this file passes did not all exist, and a new major is not
/// a promise anybody here can keep.
fn supported(version: (u32, u32, u32)) -> bool {
    version >= MIN_VERSION && version.0 < UNSUPPORTED_MAJOR
}

/// A control command that failed, said as the step it was part of.
///
/// The step is in the message because a create is six commands and "the
/// computer could not be made" leaves the operator nothing to look at.
fn step_failed(step: &str, out: &CliOutput) -> ProviderError {
    ProviderError::Operation(format!(
        "{step} failed: {}; try again, and if it persists run `container system status` in \
         Terminal to see what the runtime says",
        detail(out)
    ))
}

/// Whether the runtime is saying there is no such thing, which is an answer
/// rather than a failure. The wording is a guess from the 1.2.2 sources and
/// the spike confirms it; reading a real error as an absence is what would
/// throw away a disk, so both spellings are matched and nothing looser is.
///
/// Both streams, because which one a CLI complains on is a habit rather than a
/// contract, and a "not found" printed to stdout read as a live machine.
fn missing(out: &CliOutput) -> bool {
    let said = spoken(out);
    said.contains("not found") || said.contains("no such")
}

/// Whether the runtime is refusing because the thing is already there.
///
/// Names are derived from the computer's id, so a create that made the network
/// and then could not unmake it leaves a name that is never free again: without
/// this, that one computer could never be made, and the operator's only symptom
/// is a machine that refuses to appear. Deliberately loose, because Apple's
/// wording is unconfirmed until the spike, and safe to be: the worst a false
/// positive does is let the next step fail on its own terms.
fn already_exists(out: &CliOutput) -> bool {
    let said = spoken(out);
    said.contains("exists") || said.contains("already in use")
}

/// Everything the runtime said, lowercased, for the two questions above.
fn spoken(out: &CliOutput) -> String {
    format!("{} {}", out.stderr, out.stdout_str()).to_lowercase()
}

/// Not a Mac at all, which is the one answer that needs no process. Anything
/// finer than the operating system — the chip, the OS version — is left to
/// whether the binary is there and what version it says it is, because those
/// are answers about the machine rather than about this build.
fn unsupported_platform() -> ProviderStatus {
    ProviderStatus {
        state: ProviderReadiness::Unsupported,
        can_start: false,
        detail: "Apple Container runs only on macOS. Add an E2B API key in settings to give \
                 agents a computer instead."
            .to_string(),
    }
}

fn not_installed() -> ProviderStatus {
    ProviderStatus {
        state: ProviderReadiness::NotInstalled,
        can_start: false,
        detail: INSTALL_HINT.to_string(),
    }
}

/// It is installed and did not answer the one question every path starts with.
///
/// The runtime's own words are deliberately not forwarded here: the CLI helper
/// ends a timeout with "check the computer provider's status in Settings", and
/// this *is* that status, so quoting it would send the operator in a circle.
fn version_unanswered(err: &CliError) -> ProviderStatus {
    let detail = match err {
        CliError::Timeout { secs, .. } => format!(
            "Apple Container did not answer `container --version` within {secs}s; run `container \
             system status` in Terminal, and restart the service if it hangs."
        ),
        _ => format!(
            "Apple Container could not be asked which version it is ({err}); run `container \
             system status` in Terminal, and restart the service if it hangs."
        ),
    };
    ProviderStatus { state: ProviderReadiness::Error, can_start: false, detail }
}

fn version_unreadable(spoke: &CliOutput) -> ProviderStatus {
    ProviderStatus {
        state: ProviderReadiness::Error,
        can_start: false,
        detail: format!(
            "`container --version` did not say which version it is; it printed {:?}. Reinstall \
             the signed package from github.com/apple/container/releases.",
            detail(spoke)
        ),
    }
}

fn version_out_of_range(found: (u32, u32, u32)) -> ProviderStatus {
    let (major, minor, patch) = found;
    let (min_major, min_minor, min_patch) = MIN_VERSION;
    ProviderStatus {
        state: ProviderReadiness::Unsupported,
        can_start: false,
        detail: format!(
            "Apple Container {major}.{minor}.{patch} is installed, and this build drives \
             {min_major}.{min_minor}.{min_patch} up to but not including {UNSUPPORTED_MAJOR}.0.0. \
             Install a version in that range from github.com/apple/container/releases."
        ),
    }
}

fn service_stopped() -> ProviderStatus {
    ProviderStatus {
        state: ProviderReadiness::NotRunning,
        can_start: true,
        detail: "Apple Container is installed but stopped. Starting a computer will start its \
                 service."
            .to_string(),
    }
}

/// Installed, and its service did not answer at all — which is a service that
/// is wedged rather than one that is down, and worth saying so: starting a
/// computer still tries, and if it does not take, the operator now knows the
/// difference before they start looking.
fn service_silent() -> ProviderStatus {
    ProviderStatus {
        state: ProviderReadiness::NotRunning,
        can_start: true,
        detail: "Apple Container is installed and its service did not answer. Starting a computer \
                 will try to start it; if that does not take, run `container system stop` and \
                 `container system start` in Terminal."
            .to_string(),
    }
}

/// A status this build cannot make a machine under, as the refusal it implies.
///
/// The sentence is carried over whole. Only the variant differs, because what
/// the caller does with it differs: `Unsupported` is never worth retrying,
/// `Unconfigured` is answered by installing something, and anything else is a
/// message for a person.
///
/// One consequence worth knowing: a `--version` that timed out arrives here as
/// `Error` and leaves as `Operation`, not `Timeout`. That is deliberate —
/// `Timeout` is the variant that says a command may still be running on a
/// machine, and there is no machine yet — but it means a runtime wedged at the
/// version check is reported as an operation failure carrying the wedged
/// runtime's own next step.
fn refusal(status: ProviderStatus) -> ProviderError {
    match status.state {
        ProviderReadiness::Unsupported => ProviderError::Unsupported(status.detail),
        ProviderReadiness::NotInstalled => ProviderError::Unconfigured(status.detail),
        _ => ProviderError::Operation(status.detail),
    }
}

/// What the runtime actually said, for a message to a person: whichever stream
/// carried it, first line, bounded. Stderr first, because this is only ever
/// asked about a command that failed.
fn detail(out: &CliOutput) -> String {
    let spoken = if out.stderr.trim().is_empty() { out.stdout_str() } else { out.stderr.clone() };
    match spoken.lines().find(|line| !line.trim().is_empty()) {
        Some(line) => line.trim().chars().take(200).collect(),
        None => "it said nothing".to_string(),
    }
}

impl From<CliError> for ProviderError {
    /// Each of the three is a different next step. A deadline in particular is
    /// its own outcome: the machine is fine and the work may still be running
    /// on it, so it must never reach a caller as something to replace.
    fn from(err: CliError) -> Self {
        let message = err.to_string();
        match err {
            CliError::Spawn(_) => ProviderError::Unavailable(message),
            CliError::Timeout { .. } => ProviderError::Timeout(message),
            CliError::Io(_) => ProviderError::Operation(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::domain::ids::AgentId;

    fn creation(idle_seconds: u32) -> CreateComputer {
        CreateComputer {
            computer: ComputerId::new(),
            agent: AgentId::new(),
            agent_name: "Manager".into(),
            idle_seconds,
        }
    }

    fn command(env: BTreeMap<String, String>) -> ExecRequest {
        ExecRequest {
            argv: argv(&["/bin/bash", "-l", "-c", "echo $TOKEN"]),
            env,
            cwd: GUEST_HOME.into(),
            timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn the_service_is_started_in_the_form_that_does_not_stop_to_ask() {
        // Without the flag, a first run prompts for permission to install a
        // kernel. A spawned child has no terminal to answer with, so it hangs
        // until the timeout and the operator is told the runtime is wedged.
        assert_eq!(version_argv(), ["--version"]);
        assert_eq!(status_argv(), ["system", "status"]);
        assert_eq!(start_service_argv(), ["system", "start", "--enable-kernel-install"]);
    }

    #[test]
    fn an_image_is_looked_for_before_it_is_fetched() {
        assert_eq!(image_present_argv("img:1"), ["image", "inspect", "img:1"]);
        assert_eq!(pull_argv("img:1"), ["image", "pull", "img:1"]);
    }

    #[test]
    fn a_network_and_a_volume_are_made_labelled_and_quota_ed() {
        // The label is the ownership key, not the name: two installs on one Mac
        // make resources named identically, and the sweep deletes what it
        // believes it owns.
        let computer = ComputerId::new();
        let name = resource_name(computer);
        let installation = "guac.installation=inst-7".to_string();
        let owner = format!("guac.computer={computer}");

        assert_eq!(
            network_create_argv("inst-7", computer),
            [
                "network",
                "create",
                "--label",
                "guac=true",
                "--label",
                installation.as_str(),
                "--label",
                owner.as_str(),
                name.as_str(),
            ]
        );
        assert_eq!(
            volume_create_argv("inst-7", computer),
            [
                "volume",
                "create",
                "-s",
                "20G",
                "--label",
                "guac=true",
                "--label",
                installation.as_str(),
                "--label",
                owner.as_str(),
                name.as_str(),
            ],
            "the quota is enforced when the volume is made or never"
        );

        assert_eq!(network_delete_argv(&name), ["network", "delete", name.as_str()]);
        assert_eq!(volume_delete_argv(&name), ["volume", "delete", name.as_str()]);

        // Asked only of a name that is already taken, to find out whose it is.
        assert_eq!(network_inspect_argv(&name), ["network", "inspect", name.as_str()]);
        assert_eq!(volume_inspect_argv(&name), ["volume", "inspect", name.as_str()]);
    }

    #[test]
    fn a_resource_is_only_ours_when_it_says_so() {
        // The name proves nothing: it is eight hex characters of a random id,
        // and adopting on the strength of it would mount a stranger's home
        // volume into an agent's machine and delete it on the way out.
        let computer = ComputerId::new();
        let labelled = |owner: &str| {
            serde_json::json!([{
                "configuration": {
                    "id": "guac-x",
                    "labels": {"guac": "true", "guac.installation": "inst-7", "guac.computer": owner},
                },
            }])
        };

        assert!(left_by(&labelled(&computer.to_string()), computer));
        assert!(
            !left_by(&labelled(&ComputerId::new().to_string()), computer),
            "another computer's, even in this same installation"
        );

        // Everything unreadable is somebody else's: absent labels, an empty
        // description, a shape this build does not know.
        assert!(!left_by(&serde_json::json!([{"configuration": {"id": "guac-x"}}]), computer));
        assert!(!left_by(&serde_json::json!([]), computer));
        assert!(!left_by(&serde_json::json!({}), computer));
        assert!(!left_by(&serde_json::json!("guac-x"), computer));

        // Whether inspect answers with an array or a bare object is unconfirmed
        // until the spike, so a correctly labelled object is ours either way.
        assert!(left_by(
            &serde_json::json!({"configuration": {"labels": {"guac.computer": computer.to_string()}}}),
            computer
        ));
    }

    #[test]
    fn a_name_held_by_a_stranger_says_which_name_and_how_to_clear_it() {
        // This build will not delete something it cannot prove is its own, so
        // the way out is the operator's and the message has to hand it over.
        let err = name_taken("home volume", "guac-abcd1234", "container volume delete");
        let message = err.to_string();

        assert!(matches!(err, ProviderError::Operation(_)));
        assert!(message.contains("home volume"), "{message}");
        assert!(message.contains("guac-abcd1234"), "which name: {message}");
        assert!(
            message.contains("container volume delete guac-abcd1234"),
            "and the command that clears it: {message}"
        );
    }

    #[test]
    fn a_container_is_created_bounded_labelled_and_told_how_long_idle_is() {
        let request = creation(900);
        let name = resource_name(request.computer);
        let mount = format!("type=volume,source={name},target=/home/user");
        let installation = "guac.installation=inst-7".to_string();
        let computer = format!("guac.computer={}", request.computer);
        let agent = format!("guac.agent={}", request.agent);

        assert_eq!(
            container_create_argv(&request, "inst-7", "img:1"),
            [
                "create",
                "--name",
                name.as_str(),
                // Its own network, never the shared default one: two agents on
                // one network can reach each other's desktops.
                "--network",
                name.as_str(),
                // The home that survives sleeping, and the only mount there is.
                // No bind mount, so nothing of the Mac's filesystem is in here.
                "--mount",
                mount.as_str(),
                "--cpus",
                "4",
                "--memory",
                "4G",
                "--shm-size",
                "1G",
                "--cap-drop",
                "NET_ADMIN",
                "--cap-drop",
                "NET_RAW",
                "--cap-drop",
                "SYS_ADMIN",
                "--cap-drop",
                "SYS_MODULE",
                "--cap-drop",
                "SYS_PTRACE",
                "--label",
                "guac=true",
                "--label",
                installation.as_str(),
                "--label",
                computer.as_str(),
                // Diagnostic only. Never the ownership key: agents get renamed
                // and deleted, and their machines outlive both.
                "--label",
                agent.as_str(),
                "--env",
                "GUAC_IDLE_SECONDS=900",
                "img:1",
            ]
        );
    }

    #[test]
    fn the_only_value_ever_written_into_an_argument_is_the_idle_period() {
        // `--env NAME=value` is the shape that puts a credential in `ps`, in a
        // crash report and in any log that prints an argv. Exactly one variable
        // is allowed to travel that way, and it is a number the operator chose.
        let request = creation(900);
        let created = container_create_argv(&request, "inst-7", "img:1");
        let values: Vec<&String> = created
            .iter()
            .zip(created.iter().skip(1))
            .filter(|(flag, _)| *flag == "--env")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(values, [&"GUAC_IDLE_SECONDS=900".to_string()]);

        let env = BTreeMap::from([
            ("TOKEN".to_string(), "ghp_sentinel".to_string()),
            ("AWS_REGION".to_string(), "us-east-1".to_string()),
        ]);
        let ran = exec_argv("guac-abcd1234", &command(env));
        for (flag, value) in ran.iter().zip(ran.iter().skip(1)) {
            if flag == "--env" {
                assert!(
                    !value.contains('='),
                    "a command's variables are named, never given: {value}"
                );
            }
        }
    }

    #[test]
    fn a_command_names_its_variables_in_a_fixed_order_and_never_their_values() {
        // Sorted, because the map is sorted: an argv that changes with the
        // insertion order of a credential map is one no test can pin.
        let env = BTreeMap::from([
            ("TOKEN".to_string(), "ghp_sentinel".to_string()),
            ("AWS_REGION".to_string(), "us-east-1".to_string()),
        ]);
        let ran = exec_argv("guac-abcd1234", &command(env));

        assert_eq!(
            ran,
            [
                "exec",
                "--workdir",
                "/home/user",
                "--env",
                "AWS_REGION",
                "--env",
                "TOKEN",
                "guac-abcd1234",
                "/bin/bash",
                "-l",
                "-c",
                "echo $TOKEN",
            ]
        );
        assert!(
            ran.iter().all(|part| !part.contains("ghp_sentinel")),
            "a secret value reached the argument vector: {ran:?}"
        );
    }

    #[test]
    fn a_machines_lifecycle_is_five_argument_vectors() {
        assert_eq!(start_argv("guac-x"), ["start", "guac-x"]);
        // A container that is asked to stop gets ten seconds to do it before
        // the runtime insists, which is the desktop's chance to write out the
        // browser profile the volume exists to keep.
        assert_eq!(stop_argv("guac-x"), ["stop", "--time", "10", "guac-x"]);
        // Always forced: a running container refuses to be deleted, and every
        // caller here is removing a machine on purpose, including the rollback
        // of a create that got as far as starting one.
        assert_eq!(delete_argv("guac-x"), ["delete", "--force", "guac-x"]);
        assert_eq!(inspect_argv("guac-x"), ["inspect", "guac-x"]);
        // JSON, never the table: presentation changes between releases.
        assert_eq!(list_argv(), ["ls", "--all", "--format", "json"]);
    }

    #[test]
    fn a_state_this_build_does_not_know_is_an_error_rather_than_a_dead_machine() {
        let running = serde_json::json!([{"status": "running"}]);
        assert_eq!(read_state("guac-x", &running).unwrap(), ProviderState::Running);

        // All three keep the volume, which is the whole point of sleeping.
        for asleep in ["stopped", "created", "exited"] {
            let listed = serde_json::json!([{"status": asleep}]);
            assert_eq!(read_state("guac-x", &listed).unwrap(), ProviderState::Asleep, "{asleep}");
        }

        // Nothing in the array is the runtime positively saying there is no
        // such container, which is the one answer that permits replacing a
        // disk.
        assert_eq!(read_state("guac-x", &serde_json::json!([])).unwrap(), ProviderState::Gone);

        let Err(ProviderError::Operation(message)) =
            read_state("guac-x", &serde_json::json!([{"status": "restarting"}]))
        else {
            panic!("an unfamiliar state must not be read as permission to throw a disk away");
        };
        assert!(message.contains("guac-x"), "which machine: {message}");
        assert!(message.contains("restarting"), "and what it was told: {message}");
        assert!(message.contains("destroy the computer"), "and what to do: {message}");

        // A reply that is not the array this expects is the same kind of
        // unknown, and must not read as an empty one.
        assert!(matches!(
            read_state("guac-x", &serde_json::json!({"status": "running"})),
            Err(ProviderError::Operation(_))
        ));
    }

    #[test]
    fn the_viewer_is_pointed_at_the_guests_address_without_its_prefix_length() {
        let inspect = serde_json::json!([{
            "status": "running",
            "networks": [{"address": "192.168.64.3/24", "gateway": "192.168.64.1"}],
        }]);
        assert_eq!(read_address("guac-x", &inspect).unwrap(), "192.168.64.3");

        // A container that is up but has not been given an address yet is worth
        // waiting for, not destroying.
        let addressless = serde_json::json!([{"status": "running", "networks": []}]);
        let Err(ProviderError::Operation(message)) = read_address("guac-x", &addressless) else {
            panic!("no address yet is not the same as no machine");
        };
        assert!(message.contains("wait"), "{message}");

        // And one the runtime has never heard of is the one case where making a
        // new machine is the answer.
        let Err(ProviderError::ResourceGone(message)) =
            read_address("guac-x", &serde_json::json!([]))
        else {
            panic!("a machine that is not there is gone, not broken");
        };
        assert!(message.contains("guac-x"), "{message}");
    }

    #[test]
    fn only_this_installations_own_resources_are_claimed() {
        // The sweep deletes what this returns. A second copy of Guac on the
        // same Mac has containers with the same `guac=true` and names of the
        // same shape, and claiming one would destroy somebody else's agent's
        // work.
        let listed = serde_json::json!([
            {"status": "running", "configuration": {
                "id": "guac-aaaa1111",
                "labels": {"guac": "true", "guac.installation": "inst-7", "guac.computer": "c1"},
            }},
            {"status": "stopped", "configuration": {
                "id": "guac-bbbb2222",
                "labels": {"guac": "true", "guac.installation": "inst-other", "guac.computer": "c2"},
            }},
            {"status": "running", "configuration": {"id": "someone-elses-container"}},
            {"status": "running", "configuration": {
                "id": "guac-cccc3333",
                "labels": {"guac.installation": "inst-7"},
            }},
        ]);

        assert_eq!(read_owned(&listed, "inst-7"), ["guac-aaaa1111"]);
        assert!(read_owned(&serde_json::json!([]), "inst-7").is_empty());
        // A reply in a shape this build cannot read owns nothing, rather than
        // everything.
        assert!(read_owned(&serde_json::json!({}), "inst-7").is_empty());
    }

    fn said(stderr: &str, stdout: &str) -> CliOutput {
        CliOutput {
            status: 1,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.to_string(),
            binary: "container".into(),
        }
    }

    #[test]
    fn a_runtime_that_will_not_answer_is_not_told_to_go_and_look_at_itself() {
        // The CLI helper ends a timeout with "check the computer provider's
        // status in Settings", and this *is* that status: forwarding its words
        // sent the operator round in a circle. Named here rather than tested
        // through a real timeout, which would be a minute of waiting.
        let status =
            version_unanswered(&CliError::Timeout { binary: "container".into(), secs: 60 });

        assert_eq!(status.state, ProviderReadiness::Error);
        assert!(!status.can_start, "there is nothing to press");
        assert!(status.detail.contains("container system status"), "{}", status.detail);
        assert!(!status.detail.contains("in Settings"), "not back to here: {}", status.detail);
        assert!(status.detail.contains("60s"), "{}", status.detail);

        // Anything else it could not be asked still names the same next step.
        let unread = version_unanswered(&CliError::Io("could not be read".into()));
        assert!(unread.detail.contains("container system status"), "{}", unread.detail);
    }

    #[test]
    fn a_service_that_is_stopped_and_one_that_says_nothing_are_told_apart() {
        // Both are answered by starting a computer, so both offer it. They read
        // differently because walking into a service that is down and one that
        // is wedged are different afternoons.
        let stopped = service_stopped();
        let silent = service_silent();

        assert_eq!((stopped.state, stopped.can_start), (ProviderReadiness::NotRunning, true));
        assert_eq!((silent.state, silent.can_start), (ProviderReadiness::NotRunning, true));
        assert!(stopped.detail.contains("stopped"), "{}", stopped.detail);
        assert!(silent.detail.contains("did not answer"), "{}", silent.detail);
        assert!(silent.detail.contains("container system stop"), "a wedged one: {}", silent.detail);
        assert_ne!(stopped.detail, silent.detail);
    }

    #[test]
    fn what_the_runtime_said_is_read_from_whichever_stream_carried_it() {
        // Which stream a CLI complains on is a habit, not a contract. Read from
        // stderr alone, a "not found" printed to stdout is a machine this build
        // believes is alive, and the row that names it is never cleared.
        assert!(missing(&said("Error: not found", "")));
        assert!(missing(&said("", "Error: no such container")));
        assert!(missing(&said("", "NOT FOUND")), "and case is the runtime's business");
        assert!(!missing(&said("XPC connection interrupted", "")));

        assert!(already_exists(&said("network guac-x already exists", "")));
        assert!(already_exists(&said("", "Error: volume exists")));
        assert!(!already_exists(&said("no space left on device", "")));
    }

    #[test]
    fn a_refusal_carries_the_status_sentence_and_the_next_step_it_implies() {
        // One sentence, whether the operator meets it in Settings or an agent
        // meets it mid-turn.
        let out_of_range = version_out_of_range((1, 1, 0));
        let refused = refusal(out_of_range.clone());
        assert!(matches!(&refused, ProviderError::Unsupported(m) if *m == out_of_range.detail));

        // Installing something is a different job from an unsupported Mac, and
        // a caller that retries would be wrong about both.
        assert!(matches!(refusal(not_installed()), ProviderError::Unconfigured(_)));
        assert!(matches!(refusal(unsupported_platform()), ProviderError::Unsupported(_)));
        assert!(matches!(
            refusal(version_unanswered(&CliError::Io("x".into()))),
            ProviderError::Operation(_)
        ));
    }

    #[test]
    fn a_version_outside_the_tested_range_is_refused_by_number() {
        // The exact wording of `container --version` is unconfirmed until the
        // spike, so this reads any line with three dotted numbers in it.
        assert_eq!(
            parse_version("container CLI version 1.2.2 (build: release, commit: 0e2d8bc)"),
            Some((1, 2, 2))
        );
        assert_eq!(parse_version("1.2.2"), Some((1, 2, 2)));
        assert_eq!(parse_version("container"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("version 2"), None, "two numbers is not a version");

        assert!(supported(MIN_VERSION));
        assert!(supported((1, 2, 9)) && supported((1, 3, 0)) && supported((1, 10, 0)));
        assert!(!supported((1, 2, 1)), "the first tested release is the floor");
        assert!(!supported((1, 1, 0)));
        assert!(!supported((2, 0, 0)), "a new major is not a promise this build can keep");
    }
}

/// Everything below drives a fake `container` on disk: a `/bin/sh` script that
/// records the argument vector it was given and answers as the test needs. It
/// is the only way to assert what this file actually spawns, which is where a
/// secret would leak.
#[cfg(all(test, unix))]
mod fake_runtime {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::domain::ids::AgentId;

    /// Prints its arguments one per line, a marker, then its whole environment,
    /// so the two halves can be read separately. That separation is the claim:
    /// a secret is in the second and never the first.
    const REPORTER: &str =
        "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done\necho ---ENV---\nenv | sort\n";

    const VERSION_1_2_2: &str = "echo 'container CLI version 1.2.2 (build: release)'";

    fn write_exec(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A fake `container` that appends every invocation to `log` and exits 0,
    /// except where a rule says otherwise. A rule is matched against the whole
    /// argument vector joined by spaces, as a prefix.
    fn fake_container(dir: &Path, log: &Path, rules: &[(&str, &str)]) -> Cli {
        let mut script =
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\ncase \"$*\" in\n", log.display());
        for (pattern, body) in rules {
            script.push_str(&format!("  \"{pattern}\"*) {body} ;;\n"));
        }
        // Every path that makes a machine asks the version first, so a fixture
        // that has no opinion about it still has to answer one this build will
        // drive. Last, so a test that does have an opinion is matched first.
        if !rules.iter().any(|(pattern, _)| pattern.starts_with("--version")) {
            script.push_str(&format!("  \"--version\"*) {VERSION_1_2_2} ;;\n"));
        }
        script.push_str("esac\nexit 0\n");
        Cli::at(write_exec(dir, "container", &script))
    }

    /// On a Mac, which is what every test below is about except the two that
    /// are about the other case. Asserted rather than read from
    /// `SUPPORTED_PLATFORM`, so these run identically wherever the suite does.
    fn provider(cli: Cli) -> AppleContainer {
        AppleContainer::with_cli(cli, "inst-7", "img:1".into(), true)
    }

    /// On something that is not a Mac.
    fn provider_off_platform(cli: Cli) -> AppleContainer {
        AppleContainer::with_cli(cli, "inst-7", "img:1".into(), false)
    }

    fn log_lines(log: &Path) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(|line| line.to_string())
            .collect()
    }

    fn creation(idle_seconds: u32) -> CreateComputer {
        CreateComputer {
            computer: ComputerId::new(),
            agent: AgentId::new(),
            agent_name: "Manager".into(),
            idle_seconds,
        }
    }

    fn handle(name: &str) -> ProviderHandle {
        ProviderHandle {
            computer: ComputerId::new(),
            provider_id: name.to_string(),
            control_secret: Secret::default(),
            viewer_secret: Secret::default(),
        }
    }

    #[tokio::test]
    async fn a_runtime_that_is_not_installed_says_what_it_needs_and_where_to_get_it() {
        // This message carries the hardware requirement, because this is where
        // it is actually known: the package installs on nothing but macOS 26 on
        // Apple silicon, so an absent binary is the honest place to say so.
        // Claiming it from `cfg!(target_arch)` instead was a build that called
        // this Mac unsupported while the runtime on it worked.
        //
        // Apple Container is also in no package manager, so "install it"
        // without a location is a search the operator has to run.
        let dir = tempfile::tempdir().unwrap();
        let status = provider(Cli::at(dir.path().join("container"))).probe_runtime().await;

        assert_eq!(status.state, ProviderReadiness::NotInstalled);
        assert!(!status.can_start, "there is nothing to start");
        assert!(status.detail.contains("macOS 26 on Apple silicon"), "{}", status.detail);
        assert!(
            status.detail.contains("/usr/local/bin/container"),
            "where it looked: {}",
            status.detail
        );
        assert!(status.detail.contains("PATH"), "and the other place: {}", status.detail);
        assert!(status.detail.contains("github.com/apple/container/releases"), "{}", status.detail);
    }

    #[tokio::test]
    async fn a_runtime_that_is_installed_and_running_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("--version", VERSION_1_2_2), ("system status", "exit 0")],
        );

        let status = provider(cli).probe_runtime().await;

        assert_eq!(status.state, ProviderReadiness::Ready, "{}", status.detail);
        assert!(!status.can_start);
        assert_eq!(log_lines(&log), ["--version", "system status"], "a probe makes no machine");
    }

    #[tokio::test]
    async fn a_runtime_that_is_installed_and_stopped_offers_to_start_it() {
        // The difference that matters to Settings: this one is a button, and
        // "not installed" is a download.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("--version", VERSION_1_2_2), ("system status", "exit 1")],
        );

        let status = provider(cli).probe_runtime().await;

        assert_eq!(status.state, ProviderReadiness::NotRunning);
        assert!(status.can_start, "starting a computer is what starts the service");
        assert!(status.detail.contains("stopped"), "{}", status.detail);
    }

    #[tokio::test]
    async fn a_version_outside_the_range_names_the_version_and_the_range() {
        // An operator on 1.1 has to know which way to move and how far, because
        // both directions exist: this build refuses 2.x as well.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("--version", "echo 'container CLI version 1.1.0'")],
        );

        let status = provider(cli).probe_runtime().await;

        assert_eq!(status.state, ProviderReadiness::Unsupported);
        assert!(status.detail.contains("1.1.0"), "what is installed: {}", status.detail);
        assert!(status.detail.contains("1.2.2"), "and what is needed: {}", status.detail);
        assert_eq!(log_lines(&log), ["--version"], "a version it cannot drive is not asked more");
    }

    #[tokio::test]
    async fn a_runtime_that_will_not_say_its_version_is_an_error_not_an_absence() {
        // It answered, so it is installed; this build simply cannot read what
        // it said. Reported as itself rather than as "not installed", which
        // would send the operator to reinstall something that is already there.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli =
            fake_container(dir.path(), &log, &[("--version", "echo 'container, unversioned'")]);

        let status = provider(cli).probe_runtime().await;

        assert_eq!(status.state, ProviderReadiness::Error);
        assert!(status.detail.contains("unversioned"), "quote it: {}", status.detail);
    }

    #[tokio::test]
    async fn a_machine_that_is_not_a_mac_is_unsupported_whatever_is_installed() {
        // The operating system is the whole of the compile-time question, and
        // it is answered without spawning anything. Nothing finer is claimed
        // here: a `container` on the box is what settles the rest, because the
        // architecture of this binary says nothing about the one it spawns.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(dir.path(), &log, &[("system status", "exit 0")]);

        let status = provider_off_platform(cli).probe().await;

        assert_eq!(status.state, ProviderReadiness::Unsupported);
        assert!(status.detail.contains("only on macOS"), "{}", status.detail);
        assert!(
            !status.detail.contains("Apple silicon"),
            "the chip is not this build's to claim: {}",
            status.detail
        );
        assert!(status.detail.contains("E2B"), "and what would work: {}", status.detail);
        assert!(log_lines(&log).is_empty(), "nothing is spawned to find out");
    }

    #[tokio::test]
    async fn a_machine_that_is_not_a_mac_refuses_to_make_a_machine_on_it() {
        // The provider fails closed on its own rather than trusting that
        // whoever called it consulted `probe` first.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(dir.path(), &log, &[]);

        let err = provider_off_platform(cli)
            .create(&creation(900))
            .await
            .expect_err("this Mac cannot run it");

        assert!(matches!(err, ProviderError::Unsupported(_)), "{err:?}");
        assert!(err.to_string().contains("only on macOS"), "{err}");
        assert!(log_lines(&log).is_empty(), "and nothing was spawned to find out");
    }

    #[tokio::test]
    async fn a_version_this_build_cannot_drive_refuses_before_it_makes_anything() {
        // Same reasoning as the probe, reached as a refusal: an operator who
        // upgraded past the range must not end up with half a machine made by
        // flags that changed meaning.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("--version", "echo 'container CLI version 1.1.0'")],
        );

        let err = provider(cli).create(&creation(900)).await.expect_err("1.1.0 is below the floor");

        assert!(matches!(err, ProviderError::Unsupported(_)), "{err:?}");
        assert!(err.to_string().contains("1.1.0"), "what is installed: {err}");
        assert!(err.to_string().contains("1.2.2"), "and what is needed: {err}");
        assert_eq!(log_lines(&log), ["--version"], "nothing else was even asked");
    }

    #[tokio::test]
    async fn a_runtime_that_is_not_there_refuses_to_make_a_machine_and_says_where_to_get_it() {
        let dir = tempfile::tempdir().unwrap();

        let err = provider(Cli::at(dir.path().join("container")))
            .create(&creation(900))
            .await
            .expect_err("there is nothing to run");

        assert!(matches!(err, ProviderError::Unconfigured(_)), "{err:?}");
        assert!(err.to_string().contains("github.com/apple/container/releases"), "{err}");
    }

    #[tokio::test]
    async fn a_stopped_service_is_started_before_a_machine_is_made() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(dir.path(), &log, &[("system status", "exit 1")]);

        provider(cli).ensure_running().await.unwrap();

        assert_eq!(
            log_lines(&log),
            ["--version", "system status", "system start --enable-kernel-install"],
            "the version is the gate, and it is asked before the service is touched"
        );
    }

    #[tokio::test]
    async fn a_service_that_will_not_start_says_what_it_said() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("system status", "exit 1"), ("system start", "echo 'no kernel' >&2; exit 1")],
        );

        let err = provider(cli).ensure_running().await.expect_err("nothing can be made");

        assert!(matches!(err, ProviderError::Unavailable(_)), "{err:?}");
        assert!(err.to_string().contains("no kernel"), "{err}");
        assert!(err.to_string().contains("container system start"), "what to try: {err}");
    }

    #[tokio::test]
    async fn a_missing_image_is_fetched_before_anything_is_made() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(dir.path(), &log, &[("image inspect", "exit 1")]);
        let request = creation(900);
        let name = resource_name(request.computer);

        let made = provider(cli).create(&request).await.expect("everything answered");

        assert_eq!(made.provider_id, name, "the name is the handle: nothing else identifies it");
        assert_eq!(made.computer, request.computer);
        assert!(
            made.control_secret.is_empty() && made.viewer_secret.is_empty(),
            "a machine on this Mac needs no token to be reached"
        );

        let seen = log_lines(&log);
        assert_eq!(seen[0], "--version");
        assert_eq!(seen[1], "system status");
        assert_eq!(seen[2], "image inspect img:1");
        assert_eq!(seen[3], "image pull img:1");
        assert!(seen[4].starts_with("network create"), "{seen:?}");
        assert!(seen[5].starts_with("volume create"), "{seen:?}");
        assert!(seen[6].starts_with("create --name"), "{seen:?}");
        assert_eq!(seen[7], format!("start {name}"));
        assert_eq!(seen.len(), 8);
    }

    #[tokio::test]
    async fn an_image_that_is_already_here_is_not_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(dir.path(), &log, &[]);

        provider(cli).create(&creation(900)).await.unwrap();

        assert!(
            !log_lines(&log).iter().any(|line| line.starts_with("image pull")),
            "a pull on every create is a gigabyte of nothing"
        );
    }

    #[tokio::test]
    async fn an_image_that_cannot_be_fetched_says_what_to_try_and_leaves_nothing_behind() {
        // The expected failure until the maintainer publishes the image, so it
        // is the one message every reviewer will read first.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("image inspect", "exit 1"), ("image pull", "echo 'no such host' >&2; exit 1")],
        );

        let err = provider(cli).create(&creation(900)).await.expect_err("there is no image");

        assert!(matches!(err, ProviderError::Image(_)), "{err:?}");
        assert!(err.to_string().contains("no such host"), "{err}");
        assert!(err.to_string().contains("GUAC_COMPUTER_IMAGE"), "the way through: {err}");
        assert!(
            !log_lines(&log).iter().any(|line| line.starts_with("network create")),
            "nothing is made before there is something to boot"
        );
    }

    #[tokio::test]
    async fn a_create_that_fails_unmakes_exactly_what_it_made() {
        // A network and a volume left behind are invisible from inside the app
        // and hold 20 GiB of quota each. Unmaking more than was made is worse:
        // a forced delete of a container this call never created would destroy
        // a machine that only shares a name.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("create --name", "echo 'no space left' >&2; exit 1")],
        );
        let request = creation(900);
        let name = resource_name(request.computer);

        let err = provider(cli).create(&request).await.expect_err("the container was refused");

        assert!(matches!(err, ProviderError::Operation(_)), "{err:?}");
        assert!(err.to_string().contains("creating the computer"), "which step: {err}");
        assert!(err.to_string().contains("no space left"), "and what it said: {err}");

        let seen = log_lines(&log);
        assert_eq!(
            seen[seen.len() - 2..],
            [format!("volume delete {name}"), format!("network delete {name}")],
            "what was made is unmade newest first: {seen:?}"
        );
        assert!(
            !seen.iter().any(|line| line.starts_with("delete ")),
            "there was no container to remove: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_container_that_will_not_start_is_removed_along_with_what_it_was_given() {
        // The other half of the rollback, and the one that has something to
        // force: a container exists by this point, and an unforced delete of a
        // container the runtime thinks is running would refuse and leak it.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli =
            fake_container(dir.path(), &log, &[("start guac", "echo 'no kernel' >&2; exit 1")]);
        let request = creation(900);
        let name = resource_name(request.computer);

        let err = provider(cli).create(&request).await.expect_err("it never came up");

        assert!(err.to_string().contains("starting the computer"), "which step: {err}");
        assert!(err.to_string().contains("no kernel"), "and what it said: {err}");

        let seen = log_lines(&log);
        assert_eq!(
            seen[seen.len() - 3..],
            [
                format!("delete --force {name}"),
                format!("volume delete {name}"),
                format!("network delete {name}"),
            ],
            "all three are unmade, newest first: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_name_left_behind_by_a_failed_rollback_does_not_lock_the_computer_out_forever() {
        // A create that made the network and could not unmake it leaves a name
        // that is never free again. Treated as an obstacle, that one computer
        // could never be made, and all the operator sees is a machine that
        // refuses to appear.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let request = creation(900);
        let name = resource_name(request.computer);
        let ours = format!("printf '%s' '{}'", described_as(&request.computer.to_string()));
        let cli = fake_container(
            dir.path(),
            &log,
            &[
                ("network create", "echo 'network already exists' >&2; exit 1"),
                ("network inspect", &ours),
            ],
        );

        let made = provider(cli).create(&request).await.expect("the leftover is ours to take over");

        assert_eq!(made.provider_id, name);
        let seen = log_lines(&log);
        assert!(
            seen.contains(&format!("network inspect {name}")),
            "it asked whose the name was: {seen:?}"
        );
        assert!(
            seen.iter().any(|line| line.starts_with("volume create")),
            "it carried on: {seen:?}"
        );
        assert_eq!(seen.last().unwrap(), &format!("start {name}"));
    }

    /// What `network inspect` / `volume inspect` prints for a resource whose
    /// `guac.computer` label is `owner`.
    fn described_as(owner: &str) -> String {
        serde_json::json!([{
            "configuration": {
                "id": "guac-x",
                "labels": {"guac": "true", "guac.installation": "inst-7", "guac.computer": owner},
            },
        }])
        .to_string()
    }

    #[tokio::test]
    async fn a_volume_left_behind_is_adopted_through_its_own_inspect() {
        // The same rule down the other branch, which is worth its own test
        // because the two differ only in the argv they pass: a copy-paste
        // between them would ask about the network and adopt the volume.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let request = creation(900);
        let name = resource_name(request.computer);
        let ours = format!("printf '%s' '{}'", described_as(&request.computer.to_string()));
        let cli = fake_container(
            dir.path(),
            &log,
            &[
                ("volume create", "echo 'volume already exists' >&2; exit 1"),
                ("volume inspect", &ours),
            ],
        );

        provider(cli).create(&request).await.expect("the leftover is ours to take over");

        let seen = log_lines(&log);
        assert!(seen.contains(&format!("volume inspect {name}")), "{seen:?}");
        assert!(
            !seen.iter().any(|line| line.starts_with("network inspect")),
            "the network was made, so nothing was asked about it: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_name_held_by_another_computer_is_refused_rather_than_taken_over() {
        // The collision this guards against: eight hex characters of a random
        // id, held by another installation. Adopted, that volume is a
        // stranger's home directory mounted into this agent's machine — and
        // deleted along with it when this computer is destroyed.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let request = creation(900);
        let name = resource_name(request.computer);
        let theirs = format!("printf '%s' '{}'", described_as(&ComputerId::new().to_string()));
        let cli = fake_container(
            dir.path(),
            &log,
            &[
                ("network create", "echo 'network already exists' >&2; exit 1"),
                ("network inspect", &theirs),
            ],
        );

        let err = provider(cli).create(&request).await.expect_err("that network is not ours");

        assert!(matches!(err, ProviderError::Operation(_)), "{err:?}");
        assert!(err.to_string().contains(&name), "which name: {err}");
        assert!(
            err.to_string().contains(&format!("container network delete {name}")),
            "and how to clear it: {err}"
        );
        let seen = log_lines(&log);
        assert!(
            !seen.iter().any(|line| line.starts_with("volume create")),
            "it stopped there: {seen:?}"
        );
        assert!(
            !seen.iter().any(|line| line.starts_with("network delete")),
            "and did not delete what it could not prove was its own: {seen:?}"
        );
    }

    #[tokio::test]
    async fn a_name_whose_owner_cannot_be_read_is_refused() {
        // Fails closed. Every way of not knowing is the same answer: labels
        // that are not there, output this build cannot read, and an inspect
        // that would not answer at all.
        let unlabelled = format!("printf '%s' '{}'", serde_json::json!([{"status": "running"}]));

        for (which, description) in [
            ("no labels", unlabelled.as_str()),
            ("not json", "echo 'Warning: no kernel'"),
            ("refused", "echo 'no such network' >&2; exit 1"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("invocations");
            let cli = fake_container(
                dir.path(),
                &log,
                &[
                    ("network create", "echo 'network already exists' >&2; exit 1"),
                    ("network inspect", description),
                ],
            );

            let err = provider(cli)
                .create(&creation(900))
                .await
                .expect_err("nothing here says the name is ours");

            assert!(
                err.to_string().contains("container network delete"),
                "{which}: the operator is left without the remedy: {err}"
            );
        }
    }

    #[tokio::test]
    async fn a_secret_reaches_the_guest_through_the_environment_and_never_the_arguments() {
        // The release blocker. `--env NAME` names a variable the `container`
        // process already holds; a build that ever wrote `--env NAME=value`
        // would put an agent's credential in `ps` and in every crash report.
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::at(write_exec(dir.path(), "container", REPORTER));
        let env = BTreeMap::from([("TOKEN".to_string(), "ghp_sentinel_9x".to_string())]);

        let out = provider(cli)
            .exec(
                &handle("guac-abcd1234"),
                ExecRequest {
                    argv: argv(&["/bin/bash", "-l", "-c", "echo $TOKEN"]),
                    env,
                    cwd: GUEST_HOME.into(),
                    timeout: Duration::from_secs(10),
                },
            )
            .await
            .unwrap();

        let (args, environment) =
            out.stdout.split_once("---ENV---\n").expect("the reporter prints the marker");
        assert!(args.lines().any(|line| line == "--env"), "{args}");
        assert!(args.lines().any(|line| line == "TOKEN"), "{args}");
        assert!(!args.contains("ghp_sentinel_9x"), "the value reached the argument vector: {args}");
        assert!(
            environment.lines().any(|line| line == "TOKEN=ghp_sentinel_9x"),
            "the guest never got the credential it was to act with"
        );
        assert_eq!(out.exit_code, 0);
    }

    #[tokio::test]
    async fn a_command_that_hangs_is_killed_and_the_model_is_told_how_to_outlive_a_deadline() {
        // Two seconds against a twenty-second sleep: macOS takes its time over
        // the first execution of a file it has never seen.
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::at(write_exec(dir.path(), "container", "#!/bin/sh\nsleep 20\n"));

        let err = provider(cli)
            .exec(
                &handle("guac-abcd1234"),
                ExecRequest {
                    argv: argv(&["/bin/bash", "-l", "-c", "sleep 600"]),
                    env: BTreeMap::new(),
                    cwd: GUEST_HOME.into(),
                    timeout: Duration::from_secs(2),
                },
            )
            .await
            .expect_err("a command that outlives its deadline is not a result");

        assert!(matches!(err, ProviderError::Timeout(_)), "{err:?}");
        assert!(err.to_string().contains("nohup"), "a refusal needs a way forward: {err}");
    }

    #[tokio::test]
    async fn a_container_the_runtime_has_never_heard_of_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("inspect", "echo 'Error: not found: guac-x' >&2; exit 1")],
        );

        let state = provider(cli).inspect(&handle("guac-x")).await.unwrap();

        assert_eq!(state, ProviderState::Gone);
    }

    #[tokio::test]
    async fn a_runtime_that_will_not_answer_is_not_a_machine_that_is_gone() {
        // `Gone` is permission to throw away a disk. A wedged service says
        // something else, and reading it as an absence would replace a machine
        // that is fine.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("inspect", "echo 'XPC connection interrupted' >&2; exit 1")],
        );

        let err =
            provider(cli).inspect(&handle("guac-x")).await.expect_err("that is not an answer");

        assert!(matches!(err, ProviderError::Operation(_)), "{err:?}");
        assert!(err.to_string().contains("XPC"), "{err}");
    }

    #[tokio::test]
    async fn deleting_removes_the_container_its_volume_and_its_network_in_that_order() {
        // And tolerates whatever is already gone: a delete that failed halfway
        // is retried at the next startup, and the second attempt finds most of
        // it missing.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli =
            fake_container(dir.path(), &log, &[("volume delete", "echo 'not found' >&2; exit 1")]);

        provider(cli).delete(&handle("guac-x")).await.unwrap();

        assert_eq!(
            log_lines(&log),
            ["delete --force guac-x", "volume delete guac-x", "network delete guac-x"]
        );
    }

    #[tokio::test]
    async fn a_delete_that_actually_failed_is_reported_so_it_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(
            dir.path(),
            &log,
            &[("network delete", "echo 'network is in use' >&2; exit 1")],
        );

        let err = provider(cli).delete(&handle("guac-x")).await.expect_err("it is still there");

        assert!(err.to_string().contains("network"), "which step: {err}");
        assert!(err.to_string().contains("in use"), "and what it said: {err}");
    }

    #[tokio::test]
    async fn sleeping_and_waking_are_the_same_machine() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(dir.path(), &log, &[]);
        let held = handle("guac-x");
        let provider = provider(cli);

        provider.stop(&held).await.unwrap();
        let woken = provider.start(&held, 900).await.unwrap();

        assert_eq!(woken, held, "a local machine keeps its name and needs no new token");
        assert_eq!(log_lines(&log), ["stop --time 10 guac-x", "start guac-x"]);
    }

    #[tokio::test]
    async fn the_viewer_is_sent_to_the_guest_over_plain_tcp_with_nothing_in_its_head() {
        // Nothing is published: the address is on the private network the
        // runtime made, reachable from this Mac and nowhere else, so there is
        // no token to hold and no TLS to terminate.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let inspect = serde_json::json!([{
            "status": "running",
            "networks": [{"address": "192.168.64.3/24"}],
        }])
        .to_string();
        let body = format!("printf '%s' '{inspect}'");
        let cli = fake_container(dir.path(), &log, &[("inspect", &body)]);

        let target = provider(cli).viewer_target(&handle("guac-x"), 6080).await.unwrap();

        assert!(!target.tls);
        assert_eq!(target.host, "192.168.64.3");
        assert_eq!(target.port, 6080);
        assert!(target.headers.is_empty());
    }

    #[tokio::test]
    async fn what_this_install_owns_is_read_from_the_list_it_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let listed = serde_json::json!([
            {"configuration": {
                "id": "guac-aaaa1111",
                "labels": {"guac": "true", "guac.installation": "inst-7"},
            }},
            {"configuration": {
                "id": "guac-bbbb2222",
                "labels": {"guac": "true", "guac.installation": "inst-other"},
            }},
        ])
        .to_string();
        let body = format!("printf '%s' '{listed}'");
        let cli = fake_container(dir.path(), &log, &[("ls", &body)]);

        let owned = provider(cli).list_owned().await.unwrap();

        assert_eq!(owned, ["guac-aaaa1111"]);
        assert_eq!(log_lines(&log), ["ls --all --format json"]);
    }

    #[tokio::test]
    async fn a_list_that_is_not_json_is_an_error_naming_what_answered() {
        // The sweep deletes from this list. An unreadable answer that came back
        // empty would look exactly like an install that owns nothing.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("invocations");
        let cli = fake_container(dir.path(), &log, &[("ls", "echo 'Warning: no kernel'")]);

        let err = provider(cli).list_owned().await.expect_err("that is not a list");

        assert!(err.to_string().contains("container"), "which binary: {err}");
    }
}

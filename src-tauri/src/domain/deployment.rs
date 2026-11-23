use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Deployment {
    #[default]
    Desktop,
    Server,
}

impl Deployment {
    pub const fn capabilities(self) -> Capabilities {
        match self {
            Deployment::Desktop => Capabilities {
                loopback_endpoints: true,
                claude_provider: true,
                local_files: true,
                borrowed_signins: true,
                local_browser: false,
            },
            Deployment::Server => Capabilities {
                loopback_endpoints: true,
                claude_provider: true,
                local_files: false,
                borrowed_signins: false,
                local_browser: false,
            },
        }
    }

    pub const fn beside_a_browser(self, sidecar: bool) -> Capabilities {
        let mut here = self.capabilities();
        here.local_browser = sidecar && self.is_server();
        here
    }

    pub const fn is_server(self) -> bool {
        matches!(self, Deployment::Server)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub loopback_endpoints: bool,
    pub claude_provider: bool,
    pub local_files: bool,
    pub borrowed_signins: bool,
    pub local_browser: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Absent {
    LoopbackEndpoints,
    ClaudeProvider,
    LocalFiles,
    LocalBrowser,
}

impl Absent {
    pub const fn sentence(self) -> &'static str {
        match self {
            Absent::LoopbackEndpoints => {
                "this workspace runs on a server, so it cannot reach a model running on your own \
                 machine. Point it at an endpoint the server can reach, or run this group in a \
                 local workspace"
            }
            Absent::ClaudeProvider => {
                "this backend does not offer the Claude CLI provider. Choose an API provider \
                 or a ChatGPT subscription, or use a backend with Claude Code configured"
            }
            Absent::LocalFiles => {
                "this workspace runs on a server, so it cannot read a path on your own machine. \
                 Drop the file onto the window or pick it with the file button, and the bytes \
                 are uploaded"
            }
            Absent::LocalBrowser => {
                "this workspace has no browser of its own: Guaca runs one only beside the local \
                 host the Mac app starts. Add a Kernel API key in settings to give agents a hosted \
                 browser here, or run this group in the local workspace instead"
            }
        }
    }
}

impl std::fmt::Display for Absent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sentence())
    }
}

impl Capabilities {
    pub const fn require(self, what: Absent) -> Result<(), Absent> {
        let have = match what {
            Absent::LoopbackEndpoints => self.loopback_endpoints,
            Absent::ClaudeProvider => self.claude_provider,
            Absent::LocalFiles => self.local_files,
            Absent::LocalBrowser => self.local_browser,
        };
        if have {
            Ok(())
        } else {
            Err(what)
        }
    }
}

pub fn is_loopback(url: &str) -> bool {
    let Some(rest) = url.split_once("://").map(|(_, rest)| rest) else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = if let Some(end) = host.find(']') {
        &host[..=end]
    } else {
        host.split(':').next().unwrap_or_default()
    };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || is_loopback_v4(&host)
        || host == "host.docker.internal"
}

fn is_loopback_v4(host: &str) -> bool {
    let mut octets = host.split('.');
    let first = octets.next().and_then(|o| o.parse::<u8>().ok());
    let rest = octets.filter_map(|o| o.parse::<u8>().ok()).count();
    first == Some(127) && rest == 3 && host.split('.').count() == 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_resources_are_available_but_client_credentials_and_files_are_not() {
        let desktop = Deployment::Desktop.capabilities();
        let server = Deployment::Server.capabilities();
        for what in [Absent::LoopbackEndpoints, Absent::ClaudeProvider, Absent::LocalFiles] {
            assert_eq!(desktop.require(what), Ok(()), "a desktop has {what:?}");
            let expected = match what {
                Absent::LoopbackEndpoints | Absent::ClaudeProvider => Ok(()),
                _ => Err(what),
            };
            assert_eq!(server.require(what), expected);
        }
    }

    #[test]
    fn every_refusal_says_what_to_do_instead() {
        for what in [
            Absent::LoopbackEndpoints,
            Absent::ClaudeProvider,
            Absent::LocalFiles,
            Absent::LocalBrowser,
        ] {
            let said = what.sentence();
            assert!(said.len() > 60, "{what:?} is too short to have said anything: {said}");
            assert!(
                said.contains(" or ") || said.contains("instead") || said.contains(", and "),
                "{what:?} refuses without offering a way forward: {said}"
            );
            assert!(
                !said.starts_with(char::is_uppercase) || said.starts_with("Claude Code"),
                "{what:?} is interpolated mid-sentence and must not open with a capital: {said}"
            );
        }
    }

    #[test]
    fn a_box_may_not_offer_a_signin_its_machine_already_has() {
        assert!(Deployment::Desktop.capabilities().borrowed_signins);
        assert!(Deployment::Server.capabilities().claude_provider);
        assert!(!Deployment::Server.capabilities().borrowed_signins);
    }

    #[test]
    fn a_browser_of_its_own_is_offered_only_beside_a_host_that_has_one() {
        for deployment in [Deployment::Desktop, Deployment::Server] {
            assert!(!deployment.capabilities().local_browser, "{deployment:?}");
            assert_eq!(
                deployment.beside_a_browser(false).require(Absent::LocalBrowser),
                Err(Absent::LocalBrowser),
                "{deployment:?} with no sidecar"
            );
        }
        assert_eq!(Deployment::Server.beside_a_browser(true).require(Absent::LocalBrowser), Ok(()));
        assert!(
            !Deployment::Desktop.beside_a_browser(true).local_browser,
            "only a host the app runs in Docker has a sidecar beside it"
        );
        let server = Deployment::Server.beside_a_browser(true);
        assert_eq!(
            Capabilities { local_browser: false, ..server },
            Deployment::Server.capabilities(),
            "a sidecar changes nothing else a host can do"
        );
    }

    #[test]
    fn the_default_is_the_desktop_because_that_is_what_exists_today() {
        assert_eq!(Deployment::default(), Deployment::Desktop);
    }

    #[test]
    fn a_server_is_a_server_however_it_was_provisioned() {
        assert!(Deployment::Server.is_server());
        assert!(!Deployment::Desktop.is_server());
    }

    #[test]
    fn loopback_is_recognized_in_every_spelling_a_console_prints() {
        for url in [
            "http://localhost:1234/v1",
            "http://LOCALHOST:1234/v1",
            "http://127.0.0.1:11434/v1",
            "http://127.1.2.3:8080",
            "https://[::1]:443/v1",
            "http://0.0.0.0:8000/v1",
            "http://host.docker.internal:1234/v1",
            "http://user:pass@localhost:1234/v1",
        ] {
            assert!(is_loopback(url), "{url} is only reachable from the machine it is typed on");
        }
    }

    #[test]
    fn a_reachable_endpoint_is_not_mistaken_for_a_local_one() {
        for url in [
            "https://openrouter.ai/api/v1",
            "https://api.openai.com/v1",
            "http://192.168.1.10:1234/v1",
            "http://10.0.0.4:1234/v1",
            "https://localhost.example.com/v1",
            "https://127.0.0.1.nip.io/v1",
            "http://127.0.0.1.example.com/v1",
            "not a url at all",
        ] {
            assert!(!is_loopback(url), "{url} is reachable from anywhere and must not be refused");
        }
    }
}

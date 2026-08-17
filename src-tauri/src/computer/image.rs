//! Which image an agent's computer boots.
//!
//! One reference for the whole app, with one development override. The image
//! is published under a namespace only the maintainer can write to, so the
//! constant below is a placeholder until it is published and nothing pulls
//! successfully meanwhile; `GUAC_COMPUTER_IMAGE` is how a reviewer points the
//! app at an image they built themselves. It is deliberately an environment
//! variable rather than a setting: it exists so the feature can be tried
//! before publication, not so operators can choose what their agents run.

/// The variable a reviewer sets to run a locally built image.
pub const IMAGE_ENV: &str = "GUAC_COMPUTER_IMAGE";

/// Included verbatim, so the file it reads holds one line and no comment. A
/// file rather than a literal because the workflow that publishes the image is
/// what rewrites it, and a build that pulls one image while the release notes
/// name another is a machine nobody can reproduce.
const PINNED: &str = include_str!("../../../computer-image/IMAGE_REF");

/// What to pull, and what a computer's row records having been made from.
pub fn image_ref() -> String {
    image_ref_from(std::env::var(IMAGE_ENV).ok().as_deref())
}

/// Whether this app is running something other than the published image, which
/// Settings says out loud: an operator debugging a computer needs to know it is
/// not the image the release was tested with.
pub fn is_overridden() -> bool {
    from_env(std::env::var(IMAGE_ENV).ok().as_deref()).is_some()
}

/// Taken apart from the environment so both halves can be tested without two
/// tests racing over one process-wide variable.
fn image_ref_from(raw: Option<&str>) -> String {
    from_env(raw).unwrap_or_else(|| PINNED.trim().to_string())
}

/// An override is one only when it says something. An empty or blank variable
/// is a shell that exported it without a value, and reading that as an image
/// reference is a pull of `""` and an error naming nothing.
fn from_env(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_reference_is_one_line_and_names_a_tag() {
        // `include_str!` takes the file exactly as it is, so a comment or a
        // second line in it becomes part of the image reference and every pull
        // fails on a name nobody typed.
        let published = image_ref_from(None);
        assert_eq!(published.lines().count(), 1, "the file is included verbatim: {published:?}");
        assert!(published.contains("guaca-computer:"), "{published}");
        assert!(!published.contains('#'), "no comment survives the include: {published}");
        assert_eq!(published.trim(), published, "the trim is the only cleaning that happens");
    }

    #[test]
    fn a_reviewer_can_point_the_app_at_an_image_they_built() {
        // The whole reason the override exists: until the maintainer publishes,
        // the pinned reference pulls nothing and this is the only way to see
        // the feature work at all.
        assert_eq!(
            image_ref_from(Some("localhost/guaca-computer:dev")),
            "localhost/guaca-computer:dev"
        );
        assert_eq!(
            image_ref_from(Some("  localhost/guaca-computer:dev\n")),
            "localhost/guaca-computer:dev",
            "a variable set from a file or a shell carries whitespace nobody meant"
        );
    }

    #[test]
    fn an_override_that_says_nothing_is_not_one() {
        // `export GUAC_COMPUTER_IMAGE=` in a profile is a variable that is set
        // and empty. Taken as a reference it is a pull of "" and an error
        // naming nothing, when the operator meant to change nothing at all.
        assert_eq!(image_ref_from(Some("")), image_ref_from(None));
        assert_eq!(image_ref_from(Some("   ")), image_ref_from(None));

        assert_eq!(from_env(None), None);
        assert_eq!(from_env(Some("  ")), None);
        assert_eq!(from_env(Some(" img:1 ")).as_deref(), Some("img:1"));
        // Settings says "overridden" from one of these and the runtime pulls
        // from the other; they have to be the same question.
        assert_eq!(is_overridden(), from_env(std::env::var(IMAGE_ENV).ok().as_deref()).is_some());
    }
}

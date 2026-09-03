use std::time::Duration;

fn is_repo_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
}

/// True if `s` is exactly two non-empty `owner/repo`-shaped segments — not
/// a registry path like `ghcr.io/org/name` (which has an extra segment and
/// would resolve to the wrong GitHub repo if guessed at).
fn looks_like_owner_repo(s: &str) -> bool {
    let mut parts = s.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), Some(b), None) => {
            !a.is_empty() && !b.is_empty() && a.chars().all(is_repo_char) && b.chars().all(is_repo_char)
        }
        _ => false,
    }
}

/// True if `s` (optionally `v`-prefixed) looks like a version number:
/// starts with a digit, rest is alphanumeric/`.`/`-`.
fn looks_like_version(s: &str) -> bool {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_digit() => chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'),
        _ => false,
    }
}

/// Parses a Renovate-style commit title — e.g. "chore(deps): update
/// lissy93/dashy docker tag to v4.6.9" — into `("lissy93/dashy",
/// "4.6.9")`. Returns `None` whenever the title doesn't clearly encode
/// both an `owner/repo` pair and a version (e.g. a helm chart bump with
/// no repo path, or a registry path like `ghcr.io/org/name` that isn't a
/// bare GitHub `owner/repo`) — skipping silently beats guessing wrong.
pub fn parse_version_bump(message: &str) -> Option<(String, String)> {
    let title = message.lines().next()?;
    let words: Vec<&str> = title.split_whitespace().collect();
    let version = words.last().filter(|w| looks_like_version(w))?;
    let version = version.trim_start_matches('v').to_string();
    let owner_repo = words.iter().find(|w| looks_like_owner_repo(w))?.to_string();
    Some((owner_repo, version))
}

fn fetch_release_body(owner_repo: &str, tag: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{owner_repo}/releases/tags/{tag}");
    let response = ureq::get(&url)
        .set("User-Agent", "gpick")
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(5))
        .call()
        .ok()?;
    let json: serde_json::Value = response.into_json().ok()?;
    let body = json.get("body")?.as_str()?.trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// Fetches the GitHub release notes body for `owner/repo` at `version`,
/// trying both a `v`-prefixed and bare tag name (repos are inconsistent
/// about which they use). Unauthenticated — public repos only — and
/// returns `None` on any failure (network, 404, rate limit, empty body)
/// rather than erroring, since this is best-effort enrichment.
pub fn fetch_release_notes(owner_repo: &str, version: &str) -> Option<String> {
    fetch_release_body(owner_repo, &format!("v{version}")).or_else(|| fetch_release_body(owner_repo, version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_docker_tag_bump_with_owner_repo() {
        assert_eq!(
            parse_version_bump("chore(deps): update lissy93/dashy docker tag to v4.6.9"),
            Some(("lissy93/dashy".to_string(), "4.6.9".to_string()))
        );
    }

    #[test]
    fn parses_a_dependency_update_without_the_word_docker() {
        assert_eq!(
            parse_version_bump("chore(deps): update dependency baarde/cert-manager-webhook-ovh to v0.3.1"),
            Some(("baarde/cert-manager-webhook-ovh".to_string(), "0.3.1".to_string()))
        );
    }

    #[test]
    fn returns_none_for_a_helm_release_bump_with_no_repo_path() {
        assert_eq!(parse_version_bump("chore(deps): update helm release grafana to v13.1.0"), None);
    }

    #[test]
    fn returns_none_for_a_registry_path_with_an_extra_segment() {
        assert_eq!(
            parse_version_bump("chore(deps): update registry.k8s.io/external-dns/external-dns docker tag to v0.14.0"),
            None
        );
    }

    #[test]
    fn returns_none_for_a_non_version_bump_message() {
        assert_eq!(parse_version_bump("feat(linkding): add self-hosted bookmark manager"), None);
    }

    #[test]
    fn fetch_release_notes_gets_real_content_for_a_known_public_release() {
        let notes = fetch_release_notes("koenkk/zigbee2mqtt", "2.14.0");
        assert!(notes.is_some_and(|n| !n.is_empty()), "expected real release notes from the GitHub API");
    }

    #[test]
    fn fetch_release_notes_is_none_for_a_repo_with_no_such_tag() {
        assert_eq!(fetch_release_notes("lissy93/dashy", "0.0.0-does-not-exist"), None);
    }
}

use std::time::Duration;

fn is_repo_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
}

/// Takes the last two `/`-separated segments of `s` as an `owner/repo`
/// guess — handles both a bare `owner/repo` and a registry-prefixed path
/// like `ghcr.io/owner/repo` or `lscr.io/owner/repo`, which is how most
/// self-hosted app images happen to mirror their GitHub repo. A wrong
/// guess (e.g. `registry.k8s.io/external-dns/external-dns`, whose real
/// repo is `kubernetes-sigs/external-dns`) just 404s later — never shows
/// content, so guessing wrong here is safe, only guessing nothing costs.
fn extract_owner_repo(s: &str) -> Option<String> {
    let segments: Vec<&str> = s.split('/').collect();
    if segments.len() < 2 {
        return None;
    }
    let owner = segments[segments.len() - 2];
    let repo = segments[segments.len() - 1];
    let valid = |s: &str| !s.is_empty() && s.chars().all(is_repo_char);
    (valid(owner) && valid(repo)).then(|| format!("{owner}/{repo}"))
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
/// ghcr.io/authelia/authelia docker tag to v4.39.22" — into
/// `("authelia/authelia", "4.39.22")`. Returns `None` whenever the title
/// has no `/`-containing token at all (e.g. a helm chart bump with just a
/// bare chart name) or no trailing version.
pub fn parse_version_bump(message: &str) -> Option<(String, String)> {
    let title = message.lines().next()?;
    let words: Vec<&str> = title.split_whitespace().collect();
    let version = words.last().filter(|w| looks_like_version(w))?;
    let version = version.trim_start_matches('v').to_string();
    let owner_repo = words.iter().find_map(|w| extract_owner_repo(w))?;
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
    fn parses_a_registry_prefixed_docker_tag_by_taking_the_last_two_segments() {
        assert_eq!(
            parse_version_bump("chore(deps): update ghcr.io/authelia/authelia docker tag to v4.39.22"),
            Some(("authelia/authelia".to_string(), "4.39.22".to_string()))
        );
        assert_eq!(
            parse_version_bump("chore(deps): update lscr.io/linuxserver/code-server docker tag to v4.1.0"),
            Some(("linuxserver/code-server".to_string(), "4.1.0".to_string()))
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

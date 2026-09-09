use std::collections::BTreeMap;
use std::time::Duration;

use maki_core::analysis::ProjectExternalLink;
use maki_core::{ExternalLinkCheck, Maki, ProjectDiagnostic, external_link_diagnostics};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExternalLinkCheckError {
    Status(u16),
    Transport(String),
}

impl std::fmt::Display for ExternalLinkCheckError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(status) => write!(formatter, "HTTP {status}"),
            Self::Transport(message) => write!(formatter, "{message}"),
        }
    }
}

pub(crate) fn diagnostics_with_external_links(maki: &Maki) -> Vec<ProjectDiagnostic> {
    let checks = check_external_links(maki.external_links());
    maki.diagnostics_with_external_link_checks(&checks)
}

pub(crate) fn diagnostics_for_external_links(
    external_links: &[ProjectExternalLink],
) -> Vec<ProjectDiagnostic> {
    let checks = check_external_links(external_links);
    external_link_diagnostics(external_links, &checks)
}

fn check_external_links(
    external_links: &[ProjectExternalLink],
) -> BTreeMap<String, ExternalLinkCheck> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .redirects(5)
        .build();
    let mut checks = BTreeMap::new();
    for external_link in external_links {
        checks
            .entry(external_link.target.clone())
            .or_insert_with(|| check_external_link(&agent, &external_link.target));
    }
    checks
}

fn check_external_link(agent: &ureq::Agent, target: &str) -> ExternalLinkCheck {
    if !is_checkable_external_href(target) {
        return ExternalLinkCheck::Ok;
    }

    let result = match external_link_response(agent.head(target).call()) {
        Ok(()) => return ExternalLinkCheck::Ok,
        Err(ExternalLinkCheckError::Status(_)) => external_link_response(agent.get(target).call()),
        Err(error) => Err(error),
    };

    match result {
        Ok(()) => ExternalLinkCheck::Ok,
        Err(error) => ExternalLinkCheck::Broken {
            reason: error.to_string(),
        },
    }
}

fn is_checkable_external_href(target: &str) -> bool {
    let target = target.trim();
    let Some((scheme, body)) = target.split_once("://") else {
        return false;
    };
    !body.is_empty()
        && (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
}

fn external_link_response(
    response: Result<ureq::Response, ureq::Error>,
) -> Result<(), ExternalLinkCheckError> {
    match response {
        Ok(response) if response.status() < 400 => Ok(()),
        Ok(response) => Err(ExternalLinkCheckError::Status(response.status())),
        Err(ureq::Error::Status(status, _)) => Err(ExternalLinkCheckError::Status(status)),
        Err(ureq::Error::Transport(error)) => {
            Err(ExternalLinkCheckError::Transport(error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_link_policy_only_checks_http_urls_with_authorities() {
        for target in [
            "http://example.com",
            "HTTPS://example.com/path",
            " https://example.com ",
        ] {
            assert!(is_checkable_external_href(target), "{target}");
        }

        for target in [
            "mailto:me@example.com",
            "//example.com/path",
            "https:",
            "https://",
            "custom://target",
        ] {
            assert!(!is_checkable_external_href(target), "{target}");
        }
    }
}

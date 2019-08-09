//! Implements event filtering based on application release version.
//!
//! A user may configure the server to ignore certain application releases
//! (known old bad releases) and Sentry will ignore events originating from
//! clients with the specified release.

use globset::Glob;

use crate::protocol::Event;

/// Filters events generated by known problematic SDK clients.
pub fn should_filter(event: &Event, filtered_releases: &[String]) -> Result<(), String> {
    let release = event.release.value();

    if let Some(release) = release {
        for filtered_release in filtered_releases {
            if filtered_release.contains('*') {
                // we have a pattern do a glob match
                let pattern = Glob::new(filtered_release);
                if let Ok(pattern) = pattern {
                    let pattern = pattern.compile_matcher();
                    if pattern.is_match(release.as_str()) {
                        return Err("Release filtered".to_string());
                    }
                }
            } else {
                //if we don't use glob patterns just do a simple comparison
                if release.as_str() == filtered_release {
                    return Err("Release filtered".to_string());
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Event, LenientString};
    use crate::types::Annotated;

    fn get_event_for_release(release: &str) -> Event {
        Event {
            release: Annotated::from(LenientString::from(release.to_string())),
            ..Event::default()
        }
    }

    #[test]
    fn test_release_filtering() {
        let examples = &[
            //simple matches
            ("1.2.3", &["1.3.0", "1.2.3", "1.3.1"][..], true),
            ("1.2.3", &["1.3.0", "1.3.1", "1.2.3"], true),
            ("1.2.3", &["1.2.3", "1.3.0", "1.3.1"], true),
            //pattern matches
            ("1.2.3", &["1.3.0", "1.2.*", "1.3.1"], true),
            ("1.2.3", &["1.3.0", "1.3.*", "1.*"], true),
            ("1.2.3", &["*", "1.3.0", "1.3.*"], true),
            //simple non match
            ("1.2.3", &["1.3.0", "1.2.4", "1.3.1"], false),
            //pattern non match
            ("1.2.3", &["1.4.0", "1.3.*", "3.*"], false),
            //sentry compatibility tests
            ("1.2.3", &[], false),
            ("1.2.3", &["1.1.1", "1.1.2", "1.3.1"], false),
            ("1.2.3", &["1.2.3"], true),
            ("1.2.3", &["1.2.*", "1.3.0", "1.3.1"], true),
            ("1.2.3", &["1.3.0", "1.*", "1.3.1"], true),
        ];
        for &(release, blocked_releases, expected) in examples {
            let evt = get_event_for_release(release);
            let blocked_releases: Vec<_> = blocked_releases.iter().map(|r| r.to_string()).collect();
            let actual = should_filter(&evt, &blocked_releases) != Ok(());
            assert_eq!(
                actual,
                expected,
                "Release {} should have {} been filtered by {:?}",
                release,
                if expected { "" } else { "not" },
                blocked_releases
            )
        }
    }
}

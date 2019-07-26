//! Implements filtering for events originating from legacy browsers.

use regex::{Regex, RegexBuilder};
use std::collections::{BTreeSet, HashMap};

use lazy_static::lazy_static;

use semaphore_common::UA_PARSER;
use semaphore_general::protocol::{Event, Headers, Request};
use semaphore_general::types::Annotated;

use crate::actors::project::{LegacyBrowser, LegacyBrowsersFilterConfig};
use crate::event_filter::util;
use crate::event_filter::util::get_user_agent;
use uaparser::{Parser, UserAgent};

/// Filters events originating from legacy browsers
pub fn should_filter(event: &Event, config: &LegacyBrowsersFilterConfig) -> Result<(), String> {
    if !config.is_enabled || config.browsers.len() == 0 {
        return Ok(()); // globally disabled or no individual browser enabled
    }

    if let Some(user_agent_string) = get_user_agent(event) {
        let user_agent = UA_PARSER.parse_user_agent(user_agent_string);

        // remap IE Mobile to IE (sentry python, filter compatibility)
        let family = if user_agent.family == "IE Mobile" {
            "IE".into()
        } else {
            user_agent.family.to_string()
        };

        let browsers = &config.browsers;

        if browsers.contains(&LegacyBrowser::Default) {
            return default_filter(family.as_str(), &user_agent);
        }

        for browser_type in browsers {
            match browser_type {
                LegacyBrowser::IePre9 => {
                    filter_browser(family.as_str(), &user_agent, "IE", |x| x <= 8)?
                }
                LegacyBrowser::Ie9 => {
                    filter_browser(family.as_str(), &user_agent, "IE", |x| x == 9)?
                }
                LegacyBrowser::Ie10 => {
                    filter_browser(family.as_str(), &user_agent, "IE", |x| x == 10)?
                }
                LegacyBrowser::OperaMiniPre8 => {
                    filter_browser(family.as_str(), &user_agent, "Opera Mini", &|x| x < 8)?
                }
                LegacyBrowser::OperaPre15 => {
                    filter_browser(family.as_str(), &user_agent, "Opera", |x| x < 15)?
                }
                LegacyBrowser::AndroidPre4 => {
                    filter_browser(family.as_str(), &user_agent, "Android", |x| x < 4)?
                }
                LegacyBrowser::SafariPre6 => {
                    filter_browser(family.as_str(), &user_agent, "Safari", |x| x < 6)?
                }
                LegacyBrowser::Default => {
                    panic!("Browser type All should have already been handled. (programming error)")
                }
            }
        }
    }
    return Ok(());
}

lazy_static! {
    static ref MIN_VERSIONS: HashMap<&'static str, i32> = {
        let mut x = HashMap::new();
        x.insert("Chrome", 0);
        x.insert("IE", 10);
        x.insert("Firefox", 0);
        x.insert("Safari", 6);
        x.insert("Edge", 0);
        x.insert("Opera", 15);
        x.insert("Android", 4);
        x.insert("Opera Mini", 8);
        x
    };
}

fn get_browser_major_version(user_agent: &UserAgent) -> Option<i32> {
    if let Some(browser_major_version_str) = &user_agent.major {
        if let Ok(browser_major_version) = browser_major_version_str.parse::<i32>() {
            return Some(browser_major_version);
        }
    }
    return None;
}

fn default_filter(mapped_family: &str, user_agent: &UserAgent) -> Result<(), String> {
    //let family = &user_agent.family;

    if let Some(browser_major_version) = get_browser_major_version(user_agent) {
        if let Some(&min_version) = MIN_VERSIONS.get(mapped_family) {
            if min_version > browser_major_version {
                return Err("browser filtered".to_string());
            }
        }
    }

    return Ok(());
}

fn filter_browser<F>(
    mapped_family: &str,
    user_agent: &UserAgent,
    family: &str,
    should_filter: F,
) -> Result<(), String>
where
    F: FnOnce(i32) -> bool,
{
    if mapped_family == family {
        if let Some(browser_major_version) = get_browser_major_version(user_agent) {
            if should_filter(browser_major_version) {
                return Err("browser filtered".to_string());
            }
        }
    }
    return Ok(());
}

#[cfg(test)]
mod tests {
    const IE8_UA: &str = "Mozilla/4.0 (compatible; MSIE 8.0; Windows NT 6.0; Trident/4.0)";
    const IE_MOBILE9_UA: &str =
        "Mozilla/5.0 (compatible; MSIE 9.0; Windows Phone OS 7.5; Trident/5.0; IEMobile/9.0; NOKIA; Lumia 710)";
    const IE9_UA: &str = "Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 7.1; Trident/5.0)";
    const IE10_UA: &str = "Mozilla/5.0 (compatible; MSIE 10.0; Windows NT 6.1; Trident/6.0)";
    const IE11_UA: &str = "Mozilla/5.0 (Windows NT 6.3; Trident/7.0; rv:11.0) like Gecko";
    const OPERA_MINI_PRE8_UA: &str =
        "Opera/9.80 (J2ME/MIDP; Opera Mini/7.0.32796/59.323; U; fr) Presto/2.12.423 Version/12.16";
    const OPERA_MINI_8_UA: &str =
        "Opera/9.80 (J2ME/MIDP; Opera Mini/8.0.35158/36.2534; U; en) Presto/2.12.423 Version/12.16";
    const OPERA_PRE15_UA: &str =
        "Mozilla/5.0 (Windows NT 6.1; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/27.0.1453.12 Safari/537.36 OPR/14.0.1116.4";
    const OPERA_15_UA: &str =
        "Mozilla/5.0 (Windows NT 6.1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/28.0.1500.45 Safari/537.36 OPR/15.0.1147.61 (Edition Next)";
    const ANDROID_PRE4_UA: &str =
        "Mozilla/5.0 (Linux; U; Android 3.2; nl-nl; GT-P6800 Build/HTJ85B) AppleWebKit/534.13 (KHTML, like Gecko) Version/4.0 Safari/534.13";
    const ANDROID_4_UA: &str =
        "Mozilla/5.0 (Linux; U; Android 4.1.1; en-gb; Build/KLP) AppleWebKit/534.30 (KHTML, like Gecko) Version/4.0 Safari/534.30";
    const SAFARI_PRE6_UA: &str =
        "Mozilla/5.0 (Macintosh; U; Intel Mac OS X 1063; tr-DE) AppleWebKit/533.16 (KHTML like Gecko) Version/5.0 Safari/533.16";
    const SAFARI_6_UA: &str =
        "Mozilla/5.0 (Macintosh; U; Intel Mac OS X 10.17.4; en-GB) AppleWebKit/605.1.5 (KHTML, like Gecko) Version/6.0 Safari/605.1.5";

    use super::*;
    use crate::event_filter::util::test_utils::*;

    fn get_legacy_browsers_filter_config(
        is_enabled: bool,
        legacy_browsers: &[LegacyBrowser],
    ) -> LegacyBrowsersFilterConfig {
        return LegacyBrowsersFilterConfig {
            is_enabled,
            browsers: {
                let mut x = BTreeSet::<LegacyBrowser>::new();
                for elm in legacy_browsers {
                    x.insert(*elm);
                }
                x
            },
        };
    }

    #[test]
    fn it_should_not_filter_events_if_filter_is_disabled() {
        let evt = get_event_with_user_agent(IE8_UA);
        let filter_result = should_filter(
            &evt,
            &get_legacy_browsers_filter_config(false, &[LegacyBrowser::Default]),
        );
        assert_eq!(
            filter_result,
            Ok(()),
            "Event filtered although filter should have been disabled"
        )
    }

    #[test]
    fn it_should_filter_default_browsers_when_default_option_is_specified() {
        for old_user_agent in &[
            IE9_UA,
            IE_MOBILE9_UA,
            SAFARI_PRE6_UA,
            OPERA_PRE15_UA,
            ANDROID_PRE4_UA,
            OPERA_MINI_PRE8_UA,
        ] {
            let evt = get_event_with_user_agent(old_user_agent);
            let filter_result = should_filter(
                &evt,
                &get_legacy_browsers_filter_config(true, &[LegacyBrowser::Default]),
            );
            assert_ne!(
                filter_result,
                Ok(()),
                "Default filter should have filtered User Agent\n{}",
                old_user_agent
            )
        }
    }

    #[test]
    fn default_filter_should_not_filter_browsers_above_the_minimum_required_versions() {
        for old_user_agent in &[
            IE10_UA,
            SAFARI_6_UA,
            OPERA_15_UA,
            ANDROID_4_UA,
            OPERA_MINI_8_UA,
        ] {
            let evt = get_event_with_user_agent(old_user_agent);
            let filter_result = should_filter(
                &evt,
                &get_legacy_browsers_filter_config(true, &[LegacyBrowser::Default]),
            );
            assert_eq!(
                filter_result,
                Ok(()),
                "Default filter shouldn't have filtered User Agent\n{}",
                old_user_agent
            )
        }
    }

    #[test]
    fn it_should_filter_the_events_from_browsers_that_are_configured() {
        for (ref user_agent, ref active_filters) in &[
            (
                IE10_UA,
                &[LegacyBrowser::AndroidPre4, LegacyBrowser::Ie10][..],
            ),
            (IE10_UA, &[LegacyBrowser::Ie10][..]),
            (IE9_UA, &[LegacyBrowser::Ie9][..]),
            (IE_MOBILE9_UA, &[LegacyBrowser::Ie9][..]),
            (
                IE9_UA,
                &[LegacyBrowser::AndroidPre4, LegacyBrowser::Ie9][..],
            ),
            (IE8_UA, &[LegacyBrowser::IePre9][..]),
            (
                IE8_UA,
                &[LegacyBrowser::OperaPre15, LegacyBrowser::IePre9][..],
            ),
            (OPERA_PRE15_UA, &[LegacyBrowser::OperaPre15][..]),
            (
                OPERA_PRE15_UA,
                &[LegacyBrowser::Ie10, LegacyBrowser::OperaPre15][..],
            ),
            (OPERA_MINI_PRE8_UA, &[LegacyBrowser::OperaMiniPre8][..]),
            (
                OPERA_MINI_PRE8_UA,
                &[LegacyBrowser::Ie10, LegacyBrowser::OperaMiniPre8][..],
            ),
            (ANDROID_PRE4_UA, &[LegacyBrowser::AndroidPre4][..]),
            (
                ANDROID_PRE4_UA,
                &[LegacyBrowser::Ie10, LegacyBrowser::AndroidPre4][..],
            ),
            (SAFARI_PRE6_UA, &[LegacyBrowser::SafariPre6][..]),
            (
                SAFARI_PRE6_UA,
                &[LegacyBrowser::OperaPre15, LegacyBrowser::SafariPre6][..],
            ),
        ] {
            let evt = get_event_with_user_agent(user_agent);
            let filter_result = should_filter(
                &evt,
                &get_legacy_browsers_filter_config(true, active_filters),
            );
            assert_ne!(
                filter_result,
                Ok(()),
                "Filters {:?} should have filtered User Agent\n{} for ",
                active_filters,
                user_agent
            )
        }
    }

    #[test]
    fn it_should_not_filter_the_events_from_browsers_that_are_not_configured() {
        for (ref user_agent, ref active_filter) in &[
            (IE11_UA, LegacyBrowser::Ie10),
            (IE10_UA, LegacyBrowser::Ie9),
            (IE9_UA, LegacyBrowser::IePre9),
            (OPERA_15_UA, LegacyBrowser::OperaPre15),
            (OPERA_MINI_8_UA, LegacyBrowser::OperaMiniPre8),
            (ANDROID_4_UA, LegacyBrowser::AndroidPre4),
            (SAFARI_6_UA, LegacyBrowser::SafariPre6),
        ] {
            let evt = get_event_with_user_agent(user_agent);
            let filter_result = should_filter(
                &evt,
                &get_legacy_browsers_filter_config(true, &[*active_filter]),
            );
            assert_eq!(
                filter_result,
                Ok(()),
                "Filter {:?} shouldn't have filtered User Agent\n{} for ",
                active_filter,
                user_agent
            )
        }
    }

    /// Test to ensure Sentry filter compatibility
    /// To be remove if/when Sentry backward compatibility is no longer required
    mod sentry_compatibility {
        use crate::actors::project::LegacyBrowser;
        use crate::event_filter::legacy_browsers::tests::{
            get_event_with_user_agent, get_legacy_browsers_filter_config,
        };
        use crate::event_filter::legacy_browsers::*;

        // User agents used by Sentry tests
        const ANDROID2_S_UA: &str =
            "Mozilla/5.0 (Linux; U; Android 2.3.5; en-us; HTC Vision Build/GRI40) AppleWebKit/533.1 (KHTML, like Gecko) Version/4.0 Mobile Safari/533.1";
        const ANDROID4_S_UA: &str =
            "Mozilla/5.0 (Linux; Android 4.0.4; Galaxy Nexus Build/IMM76B) AppleWebKit/535.19 (KHTML, like Gecko) Chrome/18.0.1025.133 Mobile Safari/535.19";
        const IE5_S_UA: &str =
            "Mozilla/4.0 (compatible; MSIE 5.50; Windows NT; SiteKiosk 4.9; SiteCoach 1.0)";
        const IE8_S_UA: &str =
            "Mozilla/4.0 (compatible; MSIE 8.0; Windows NT 6.1; Win64; x64; Trident/4.0; .NET CLR 2.0.50727; SLCC2; .NET CLR 3.5.30729; .NET CLR 3.0.30729; Media Center PC 6.0; MDDC; Tablet PC 2.0)";
        const IE9_S_UA: &str = "Mozilla/5.0 (Windows; U; MSIE 9.0; WIndows NT 9.0; en-US))";
        const IE_MOBILE9_S_UA: &str =
            "Mozilla/5.0 (compatible; MSIE 9.0; Windows Phone OS 7.5; Trident/5.0; IEMobile/9.0; NOKIA; Lumia 710)";
        const IE10_S_UA: &str =
            "Mozilla/5.0 (compatible; MSIE 10.0; Windows NT 7.0; InfoPath.3; .NET CLR 3.1.40767; Trident/6.0; en-IN)";
        const IE_MOBILE10_S_UA: &str =
            "Mozilla/5.0 (compatible; MSIE 10.0; Windows Phone 8.0; Trident/6.0; IEMobile/10.0; ARM; Touch; NOKIA; Lumia 520)";
        const OPERA11_S_UA: &str = "Opera/9.80 (Windows NT 5.1; U; it) Presto/2.7.62 Version/11.00";
        const OPERA_12_S_UA: &str =
            "Opera/9.80 (X11; Linux i686; Ubuntu/14.10) Presto/2.12.388 Version/12.16";
        const OPERA_15_S_UA: &str =
            "Mozilla/5.0 (X11; Linux x86_64; Debian) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/28.0.1500.52 Safari/537.36 OPR/15.0.1147.100";
        const CHROME_S_UA: &str =
            "Mozilla/5.0 (Windows NT 6.1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/41.0.2228.0 Safari/537.36";
        const EDGE_S_UA: &str =
            "Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/42.0.2311.135 Safari/537.36 Edge/12.10136";
        const SAFARI5_S_UA: &str =
            "Mozilla/5.0 (Windows; U; Windows NT 6.1; zh-HK) AppleWebKit/533.18.1 (KHTML, like Gecko) Version/5.0.2 Safari/533.18.5";
        const SAFARI7_S_UA: &str =
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_9_3) AppleWebKit/537.75.14 (KHTML, like Gecko) Version/7.0.3 Safari/7046A194A";
        const OPERA_MINI8_S_UA: &str =
            "Opera/9.80 (J2ME/MIDP; Opera Mini/8.0.35158/36.2534; U; en) Presto/2.12.423 Version/12.16";
        const OPERA_MINI7_S_UA: &str =
            "Opera/9.80 (J2ME/MIDP; Opera Mini/7.0.32796/59.323; U; fr) Presto/2.12.423 Version/12.16";

        #[test]
        fn it_should_filter_user_agents_in_the_same_way_as_sentry() {
            for (ref user_agent, ref active_filter) in &[
                (ANDROID2_S_UA, LegacyBrowser::Default),
                (IE9_S_UA, LegacyBrowser::Default),
                (IE_MOBILE9_S_UA, LegacyBrowser::Default),
                (OPERA_12_S_UA, LegacyBrowser::Default),
                (OPERA_MINI7_S_UA, LegacyBrowser::Default),
                (OPERA_12_S_UA, LegacyBrowser::OperaPre15),
                (OPERA_MINI7_S_UA, LegacyBrowser::OperaMiniPre8),
                (OPERA_MINI7_S_UA, LegacyBrowser::Default),
                (OPERA_MINI7_S_UA, LegacyBrowser::Default),
                (IE8_S_UA, LegacyBrowser::IePre9),
                (IE8_S_UA, LegacyBrowser::Default),
                (IE9_S_UA, LegacyBrowser::Ie9),
                (IE_MOBILE9_S_UA, LegacyBrowser::Ie9),
                (IE10_S_UA, LegacyBrowser::Ie10),
                (IE_MOBILE10_S_UA, LegacyBrowser::Ie10),
                (SAFARI5_S_UA, LegacyBrowser::SafariPre6),
                (SAFARI5_S_UA, LegacyBrowser::Default),
                (ANDROID2_S_UA, LegacyBrowser::AndroidPre4),
            ] {
                let evt = get_event_with_user_agent(user_agent);
                let filter_result = should_filter(
                    &evt,
                    &get_legacy_browsers_filter_config(true, &[*active_filter]),
                );
                assert_ne!(
                    filter_result,
                    Ok(()),
                    "Filter <{:?}> should have filtered User Agent\n{} for ",
                    active_filter,
                    user_agent
                )
            }
        }

        #[test]
        fn it_should_not_filter_user_agents_in_the_same_way_as_sentry() {
            for (ref user_agent, ref active_filter) in &[
                (ANDROID4_S_UA, LegacyBrowser::Default),
                (IE10_S_UA, LegacyBrowser::Default),
                (IE_MOBILE10_S_UA, LegacyBrowser::Default),
                (CHROME_S_UA, LegacyBrowser::Default),
                (EDGE_S_UA, LegacyBrowser::Default),
                (OPERA_15_S_UA, LegacyBrowser::Default),
                (IE10_S_UA, LegacyBrowser::Default),
                (IE_MOBILE10_S_UA, LegacyBrowser::Default),
                (SAFARI7_S_UA, LegacyBrowser::Default),
            ] {
                let evt = get_event_with_user_agent(user_agent);
                let filter_result = should_filter(
                    &evt,
                    &get_legacy_browsers_filter_config(true, &[*active_filter]),
                );
                assert_eq!(
                    filter_result,
                    Ok(()),
                    "Filter {:?} shouldn't have filtered User Agent\n{} for ",
                    active_filter,
                    user_agent
                )
            }
        }
    }
}

// TODO: Fix casts between RelayGeoIpLookup and GeoIpLookup
#![allow(clippy::cast_ptr_alignment)]
#![deny(unused_must_use)]

use std::ffi::CStr;
use std::os::raw::c_char;
use std::slice;

use relay_common::{glob_match_bytes, GlobOptions};
use relay_general::pii::{
    selector_suggestions_from_value, DataScrubbingConfig, PiiConfig, PiiProcessor,
};
use relay_general::processor::{process_value, split_chunks, ProcessingState};
use relay_general::protocol::{Event, VALID_PLATFORMS};
use relay_general::store::{GeoIpLookup, StoreConfig, StoreProcessor};
use relay_general::types::{Annotated, Remark};
use relay_sampling::RuleCondition;

use crate::core::{RelayBuf, RelayStr};

/// A geo ip lookup helper based on maxmind db files.
pub struct RelayGeoIpLookup;

/// The processor that normalizes events for store.
pub struct RelayStoreNormalizer;

lazy_static::lazy_static! {
    static ref VALID_PLATFORM_STRS: Vec<RelayStr> =
        VALID_PLATFORMS.iter().map(|s| RelayStr::new(s)).collect();
}

/// Chunks the given text based on remarks.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_split_chunks(
    string: *const RelayStr,
    remarks: *const RelayStr,
) -> RelayStr {
    let remarks: Vec<Remark> = serde_json::from_str((*remarks).as_str())?;
    let chunks = split_chunks((*string).as_str(), &remarks);
    let json = serde_json::to_string(&chunks)?;
    json.into()
}

/// Opens a maxminddb file by path.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_geoip_lookup_new(path: *const c_char) -> *mut RelayGeoIpLookup {
    let path = CStr::from_ptr(path).to_string_lossy();
    let lookup = GeoIpLookup::open(path.as_ref())?;
    Box::into_raw(Box::new(lookup)) as *mut RelayGeoIpLookup
}

/// Frees a `RelayGeoIpLookup`.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_geoip_lookup_free(lookup: *mut RelayGeoIpLookup) {
    if !lookup.is_null() {
        let lookup = lookup as *mut GeoIpLookup;
        Box::from_raw(lookup);
    }
}

/// Returns a list of all valid platform identifiers.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_valid_platforms(size_out: *mut usize) -> *const RelayStr {
    if let Some(size_out) = size_out.as_mut() {
        *size_out = VALID_PLATFORM_STRS.len();
    }

    VALID_PLATFORM_STRS.as_ptr()
}

/// Creates a new normalization processor.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_store_normalizer_new(
    config: *const RelayStr,
    geoip_lookup: *const RelayGeoIpLookup,
) -> *mut RelayStoreNormalizer {
    let config: StoreConfig = serde_json::from_str((*config).as_str())?;
    let geoip_lookup = (geoip_lookup as *const GeoIpLookup).as_ref();
    let normalizer = StoreProcessor::new(config, geoip_lookup);
    Box::into_raw(Box::new(normalizer)) as *mut RelayStoreNormalizer
}

/// Frees a `RelayStoreNormalizer`.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_store_normalizer_free(normalizer: *mut RelayStoreNormalizer) {
    if !normalizer.is_null() {
        let normalizer = normalizer as *mut StoreProcessor;
        Box::from_raw(normalizer);
    }
}

/// Normalizes the event given as JSON.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_store_normalizer_normalize_event(
    normalizer: *mut RelayStoreNormalizer,
    event: *const RelayStr,
) -> RelayStr {
    let processor = normalizer as *mut StoreProcessor;
    let mut event = Annotated::<Event>::from_json((*event).as_str())?;
    process_value(&mut event, &mut *processor, ProcessingState::root())?;
    RelayStr::from_string(event.to_json()?)
}

/// Replaces invalid JSON generated by Python.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_translate_legacy_python_json(event: *mut RelayStr) -> bool {
    let data = slice::from_raw_parts_mut((*event).data as *mut u8, (*event).len);
    json_forensics::translate_slice(data);
    true
}

/// Validate a PII config against the schema. Used in project options UI.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_validate_pii_config(value: *const RelayStr) -> RelayStr {
    match serde_json::from_str((*value).as_str()) {
        Ok(PiiConfig { .. }) => RelayStr::new(""),
        Err(e) => RelayStr::from_string(e.to_string()),
    }
}

/// Convert an old datascrubbing config to the new PII config format.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_convert_datascrubbing_config(config: *const RelayStr) -> RelayStr {
    let config: DataScrubbingConfig = serde_json::from_str((*config).as_str())?;
    match *config.pii_config() {
        Some(ref config) => RelayStr::from_string(config.to_json()?),
        None => RelayStr::new("{}"),
    }
}

/// Scrub an event using new PII stripping config.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_pii_strip_event(
    config: *const RelayStr,
    event: *const RelayStr,
) -> RelayStr {
    let config = serde_json::from_str::<PiiConfig>((*config).as_str())?;
    let compiled = config.compiled();
    let mut processor = PiiProcessor::new(&compiled);

    let mut event = Annotated::<Event>::from_json((*event).as_str())?;
    process_value(&mut event, &mut processor, ProcessingState::root())?;

    RelayStr::from_string(event.to_json()?)
}

/// Walk through the event and collect selectors that can be applied to it in a PII config. This
/// function is used in the UI to provide auto-completion of selectors.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_pii_selector_suggestions_from_event(
    event: *const RelayStr,
) -> RelayStr {
    let mut event = Annotated::<Event>::from_json((*event).as_str())?;
    let rv = selector_suggestions_from_value(&mut event);
    RelayStr::from_string(serde_json::to_string(&rv)?)
}

/// A test function that always panics.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_test_panic() -> () {
    panic!("this is a test panic")
}

/// Controls the globbing behaviors.
#[repr(u32)]
pub enum GlobFlags {
    /// When enabled `**` matches over path separators and `*` does not.
    DoubleStar = 1,
    /// Enables case insensitive path matching.
    CaseInsensitive = 2,
    /// Enables path normalization.
    PathNormalize = 4,
    /// Allows newlines.
    AllowNewline = 8,
}

/// Performs a glob operation on bytes.
///
/// Returns `true` if the glob matches, `false` otherwise.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_is_glob_match(
    value: *const RelayBuf,
    pat: *const RelayStr,
    flags: GlobFlags,
) -> bool {
    let mut options = GlobOptions::default();
    let flags = flags as u32;
    if (flags & GlobFlags::DoubleStar as u32) != 0 {
        options.double_star = true;
    }
    if (flags & GlobFlags::CaseInsensitive as u32) != 0 {
        options.case_insensitive = true;
    }
    if (flags & GlobFlags::PathNormalize as u32) != 0 {
        options.path_normalize = true;
    }
    if (flags & GlobFlags::AllowNewline as u32) != 0 {
        options.allow_newline = true;
    }
    glob_match_bytes((*value).as_bytes(), (*pat).as_str(), options)
}

/// Parse a sentry release structure from a string.
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_parse_release(value: *const RelayStr) -> RelayStr {
    let release = sentry_release_parser::Release::parse((*value).as_str())?;
    RelayStr::from_string(serde_json::to_string(&release)?)
}

/// Validate a dynamic rule condition.
/// We only validate conditions in Relay since conditions are the only part of the rules that are
/// volatile and need constant updating to keep in sync between Relay and Sentry
#[no_mangle]
#[relay_ffi::catch_unwind]
pub unsafe extern "C" fn relay_validate_dynamic_rule_condition(value: *const RelayStr) -> RelayStr {
    let ret_val = match serde_json::from_str::<RuleCondition>((*value).as_str()) {
        Ok(condition) => {
            if condition.supported() {
                "".to_string()
            } else {
                "invalid condition".to_string()
            }
        }
        Err(e) => e.to_string(),
    };
    RelayStr::from_string(ret_val)
}

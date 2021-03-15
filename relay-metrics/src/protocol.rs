use std::collections::BTreeMap;
use std::fmt::{self, Write};
use std::iter::FusedIterator;

use serde::{Deserialize, Serialize};

use relay_common::UnixTimestamp;

/// TODO: Doc
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurationPrecision {
    /// TODO: Doc
    NanoSecond,
    /// TODO: Doc
    MilliSecond,
    /// TODO: Doc
    Second,
}

impl fmt::Display for DurationPrecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DurationPrecision::NanoSecond => f.write_str("ns"),
            DurationPrecision::MilliSecond => f.write_str("ms"),
            DurationPrecision::Second => f.write_str("s"),
        }
    }
}

/// TODO: Doc
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricUnit {
    /// TODO: Doc
    Duration(DurationPrecision),
    /// TODO: Doc
    None,
}

impl MetricUnit {
    /// Returns `true` if the metric_unit is [`None`].
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl Default for MetricUnit {
    fn default() -> Self {
        MetricUnit::None
    }
}

impl fmt::Display for MetricUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricUnit::Duration(precision) => precision.fmt(f),
            MetricUnit::None => f.write_str(""),
        }
    }
}

impl std::str::FromStr for MetricUnit {
    type Err = ParseMetricError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "ns" => Self::Duration(DurationPrecision::NanoSecond),
            "ms" => Self::Duration(DurationPrecision::MilliSecond),
            "s" => Self::Duration(DurationPrecision::Second),
            "" | "unit" | "none" => Self::None,
            _ => return Err(ParseMetricError(())),
        })
    }
}

relay_common::impl_str_serde!(MetricUnit, "a metric unit string");

/// TODO: Doc
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MetricValue {
    /// TODO: Doc
    Integer(i64),
    /// TODO: Doc
    Float(f64),
    // TODO: Uuid(Uuid),
    /// TODO: Doc
    Custom(String),
}

impl fmt::Display for MetricValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricValue::Float(float) => float.fmt(f),
            MetricValue::Integer(int) => int.fmt(f),
            MetricValue::Custom(string) => string.fmt(f),
        }
    }
}

impl std::str::FromStr for MetricValue {
    type Err = ParseMetricError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if let Ok(int) = s.parse() {
            Self::Integer(int)
        } else if let Ok(float) = s.parse() {
            Self::Float(float)
        } else {
            Self::Custom(s.to_owned())
        })
    }
}

/// TODO: Doc
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricType {
    /// TODO: Doc
    Counter,
    /// TODO: Doc
    Histogram,
    /// TODO: Doc
    Set,
    /// TODO: Doc
    Gauge,
}

impl fmt::Display for MetricType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricType::Counter => f.write_str("c"),
            MetricType::Histogram => f.write_str("h"),
            MetricType::Set => f.write_str("s"),
            MetricType::Gauge => f.write_str("g"),
        }
    }
}

impl std::str::FromStr for MetricType {
    type Err = ParseMetricError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "c" | "m" => Self::Counter,
            "h" | "d" | "ms" => Self::Histogram,
            "s" => Self::Set,
            "g" => Self::Gauge,
            _ => return Err(ParseMetricError(())),
        })
    }
}

relay_common::impl_str_serde!(MetricType, "a metric type string");

/// TODO: Doc
#[derive(Clone, Copy, Debug)]
pub struct ParseMetricError(());

impl fmt::Display for ParseMetricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to parse metric")
    }
}

/// TODO: Doc
fn parse_name_unit(string: &str) -> Option<(String, MetricUnit)> {
    let mut components = string.split('@');
    let name = components.next()?.to_owned();
    let unit = match components.next() {
        Some(s) => s.parse().ok()?,
        None => MetricUnit::default(),
    };

    Some((name, unit))
}

/// TODO: Doc
fn parse_name_unit_value(string: &str) -> Option<(String, MetricUnit, MetricValue)> {
    let mut components = string.splitn(2, ':');
    let (name, unit) = components.next().and_then(parse_name_unit)?;
    let value = components.next().and_then(|s| s.parse().ok())?;
    Some((name, unit, value))
}

/// TODO: Doc
fn parse_tags(string: &str) -> Option<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();

    for pair in string.split(',') {
        let mut name_value = pair.splitn(2, ':');
        let name = name_value.next()?;
        let value = name_value.next().unwrap_or_default();
        map.insert(name.to_owned(), value.to_owned());
    }

    Some(map)
}

/// TODO: Doc
fn parse_timestamp(string: &str) -> Option<UnixTimestamp> {
    if let Ok(int) = string.parse() {
        Some(UnixTimestamp::from_secs(int))
    } else if let Ok(float) = string.parse::<f64>() {
        if float < 0f64 {
            None
        } else {
            Some(UnixTimestamp::from_secs(float.trunc() as u64))
        }
    } else {
        None
    }
}

/// TODO: Doc
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Metric {
    /// TODO: Doc
    pub name: String,
    /// TODO: Doc
    #[serde(default, skip_serializing_if = "MetricUnit::is_none")]
    pub unit: MetricUnit,
    /// TODO: Doc
    pub value: MetricValue,
    /// TODO: Doc
    #[serde(rename = "type")]
    pub ty: MetricType,
    /// TODO: Doc
    pub timestamp: UnixTimestamp,
    /// TODO: Doc
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
}

impl Metric {
    fn parse_str(string: &str, timestamp: UnixTimestamp) -> Option<Self> {
        let mut components = string.split('|');

        let (name, unit, value) = components.next().and_then(parse_name_unit_value)?;
        let ty = components.next().and_then(|s| s.parse().ok())?;

        let mut metric = Self {
            name,
            unit,
            value,
            ty,
            timestamp,
            tags: BTreeMap::new(),
        };

        for component in components {
            match component.chars().next() {
                Some('#') => metric.tags = parse_tags(component.get(1..)?)?,
                Some('\'') => metric.timestamp = parse_timestamp(component.get(1..)?)?,
                _ => (),
            }
        }

        Some(metric)
    }

    /// TODO: Doc
    pub fn parse(slice: &[u8], timestamp: UnixTimestamp) -> Result<Self, ParseMetricError> {
        let string = std::str::from_utf8(slice).or(Err(ParseMetricError(())))?;
        Self::parse_str(string, timestamp).ok_or(ParseMetricError(()))
    }

    /// TODO: Doc
    pub fn parse_all(slice: &[u8], timestamp: UnixTimestamp) -> ParseMetrics<'_> {
        ParseMetrics { slice, timestamp }
    }

    /// TODO: Doc
    pub fn serialize(&self) -> String {
        let mut string = self.name.clone();

        if self.unit != MetricUnit::None {
            write!(string, "@{}", self.unit).ok();
        }

        write!(string, ":{}|{}|'{}", self.value, self.ty, self.timestamp).ok();

        for (index, (key, value)) in self.tags.iter().enumerate() {
            match index {
                0 => write!(string, "|#{}", key).ok(),
                _ => write!(string, ",{}", key).ok(),
            };

            if !value.is_empty() {
                write!(string, ":{}", value).ok();
            }
        }

        string
    }
}

/// TODO: Doc
#[derive(Clone, Debug)]
pub struct ParseMetrics<'a> {
    slice: &'a [u8],
    timestamp: UnixTimestamp,
}

impl Default for ParseMetrics<'_> {
    fn default() -> Self {
        Self {
            slice: &[],
            // The timestamp will never be returned.
            timestamp: UnixTimestamp::from_secs(4711),
        }
    }
}

impl Iterator for ParseMetrics<'_> {
    type Item = Result<Metric, ParseMetricError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.slice.is_empty() {
                return None;
            }

            let mut split = self.slice.splitn(2, |&b| b == b'\n');
            let current = split.next()?;
            self.slice = split.next().unwrap_or_default();

            let string = match std::str::from_utf8(current) {
                Ok(string) => string.strip_suffix('\r').unwrap_or(string),
                Err(_) => return Some(Err(ParseMetricError(()))),
            };

            if !string.is_empty() {
                return Some(Metric::parse_str(string, self.timestamp).ok_or(ParseMetricError(())));
            }
        }
    }
}

impl FusedIterator for ParseMetrics<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_garbage() {
        let s = "x23-408j17z4232@#34d\nc3456y7^😎";
        let timestamp = UnixTimestamp::from_secs(4711);
        let result = Metric::parse(s.as_bytes(), timestamp);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_counter() {
        let s = "foo:42|c";
        let timestamp = UnixTimestamp::from_secs(4711);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        insta::assert_debug_snapshot!(metric, @r###"
        Metric {
            name: "foo",
            unit: None,
            value: Integer(
                42,
            ),
            ty: Counter,
            timestamp: UnixTimestamp(4711),
            tags: {},
        }
        "###);
    }

    #[test]
    fn test_parse_histogram() {
        let s = "foo:17.5|h";
        let timestamp = UnixTimestamp::from_secs(4711);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        insta::assert_debug_snapshot!(metric, @r###"
        Metric {
            name: "foo",
            unit: None,
            value: Float(
                17.5,
            ),
            ty: Histogram,
            timestamp: UnixTimestamp(4711),
            tags: {},
        }
        "###);
    }

    #[test]
    fn test_parse_set() {
        let s = "foo:e2546e4c-ecd0-43ad-ae27-87960e57a658|s";
        let timestamp = UnixTimestamp::from_secs(4711);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        insta::assert_debug_snapshot!(metric, @r###"
        Metric {
            name: "foo",
            unit: None,
            value: Custom(
                "e2546e4c-ecd0-43ad-ae27-87960e57a658",
            ),
            ty: Set,
            timestamp: UnixTimestamp(4711),
            tags: {},
        }
        "###);
    }

    #[test]
    fn test_parse_gauge() {
        let s = "foo:42|g";
        let timestamp = UnixTimestamp::from_secs(4711);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        insta::assert_debug_snapshot!(metric, @r###"
        Metric {
            name: "foo",
            unit: None,
            value: Integer(
                42,
            ),
            ty: Gauge,
            timestamp: UnixTimestamp(4711),
            tags: {},
        }
        "###);
    }

    #[test]
    fn test_parse_unit() {
        let s = "foo@s:17.5|h";
        let timestamp = UnixTimestamp::from_secs(4711);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        assert_eq!(metric.unit, MetricUnit::Duration(DurationPrecision::Second));
    }

    #[test]
    fn test_parse_timestamp() {
        let s = "foo:17.5|h|'1337";
        let timestamp = UnixTimestamp::from_secs(0xffff_ffff);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        assert_eq!(metric.timestamp, UnixTimestamp::from_secs(1337));
    }

    #[test]
    fn test_parse_timestamp_float() {
        let s = "foo:17.5|h|'1337.666";
        let timestamp = UnixTimestamp::from_secs(0xffff_ffff);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        assert_eq!(metric.timestamp, UnixTimestamp::from_secs(1337));
    }

    #[test]
    fn test_parse_tags() {
        let s = "foo:17.5|h|#foo,bar:baz";
        let timestamp = UnixTimestamp::from_secs(4711);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        insta::assert_debug_snapshot!(metric.tags, @r###"
        {
            "bar": "baz",
            "foo": "",
        }
        "###);
    }

    #[test]
    fn test_serialize_basic() {
        let metric = Metric {
            name: "foo".to_owned(),
            unit: MetricUnit::None,
            value: MetricValue::Integer(42),
            ty: MetricType::Counter,
            timestamp: UnixTimestamp::from_secs(4711),
            tags: BTreeMap::new(),
        };

        assert_eq!(metric.serialize(), "foo:42|c|'4711");
    }

    #[test]
    fn test_serialize_unit() {
        let metric = Metric {
            name: "foo".to_owned(),
            unit: MetricUnit::Duration(DurationPrecision::Second),
            value: MetricValue::Integer(42),
            ty: MetricType::Counter,
            timestamp: UnixTimestamp::from_secs(4711),
            tags: BTreeMap::new(),
        };

        assert_eq!(metric.serialize(), "foo@s:42|c|'4711");
    }

    #[test]
    fn test_serialize_tags() {
        let mut tags = BTreeMap::new();
        tags.insert("empty".to_owned(), "".to_owned());
        tags.insert("full".to_owned(), "value".to_owned());

        let metric = Metric {
            name: "foo".to_owned(),
            unit: MetricUnit::None,
            value: MetricValue::Integer(42),
            ty: MetricType::Counter,
            timestamp: UnixTimestamp::from_secs(4711),
            tags,
        };

        assert_eq!(metric.serialize(), "foo:42|c|'4711|#empty,full:value");
    }

    #[test]
    fn test_roundtrip() {
        let s = "foo@s:17.5|h|'1337|#bar,foo:baz";
        let timestamp = UnixTimestamp::from_secs(0xffff_ffff);
        let metric = Metric::parse(s.as_bytes(), timestamp).unwrap();
        assert_eq!(metric.serialize(), s);
    }

    #[test]
    fn test_serde_json() {
        let json = r#"{
  "name": "foo",
  "unit": "s",
  "value": 42,
  "type": "c",
  "timestamp": 4711,
  "tags": {
    "empty": "",
    "full": "value"
  }
}"#;

        let metric = serde_json::from_str::<Metric>(json).unwrap();
        insta::assert_debug_snapshot!(metric, @r###"
        Metric {
            name: "foo",
            unit: Duration(
                Second,
            ),
            value: Integer(
                42,
            ),
            ty: Counter,
            timestamp: UnixTimestamp(4711),
            tags: {
                "empty": "",
                "full": "value",
            },
        }
        "###);

        let string = serde_json::to_string_pretty(&metric).unwrap();
        assert_eq!(string, json);
    }

    #[test]
    fn test_serde_json_defaults() {
        // NB: timestamp is required in JSON as opposed to the text representation
        let json = r#"{
            "name": "foo",
            "value": 42,
            "type": "c",
            "timestamp": 4711
        }"#;

        let metric = serde_json::from_str::<Metric>(json).unwrap();
        insta::assert_debug_snapshot!(metric, @r###"
        Metric {
            name: "foo",
            unit: None,
            value: Integer(
                42,
            ),
            ty: Counter,
            timestamp: UnixTimestamp(4711),
            tags: {},
        }
        "###);
    }

    #[test]
    fn test_parse_all() {
        let s = "foo:42|c\nbar:17|c";
        let timestamp = UnixTimestamp::from_secs(4711);

        let metrics: Vec<Metric> = Metric::parse_all(s.as_bytes(), timestamp)
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(metrics.len(), 2);
    }

    #[test]
    fn test_parse_all_crlf() {
        let s = "foo:42|c\r\nbar:17|c";
        let timestamp = UnixTimestamp::from_secs(4711);

        let metrics: Vec<Metric> = Metric::parse_all(s.as_bytes(), timestamp)
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(metrics.len(), 2);
    }

    #[test]
    fn test_parse_all_empty_lines() {
        let s = "foo:42|c\n\n\nbar:17|c";
        let timestamp = UnixTimestamp::from_secs(4711);

        let metric_count = Metric::parse_all(s.as_bytes(), timestamp).count();
        assert_eq!(metric_count, 2);
    }

    #[test]
    fn test_parse_all_trailing() {
        let s = "foo:42|c\nbar:17|c\n";
        let timestamp = UnixTimestamp::from_secs(4711);

        let metric_count = Metric::parse_all(s.as_bytes(), timestamp).count();
        assert_eq!(metric_count, 2);
    }
}
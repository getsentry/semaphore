use std::collections::BTreeMap;

use itertools::Itertools;
use regex::RegexBuilder;

use crate::datascrubbing::DataScrubbingConfig;
use crate::pii::{Pattern, PiiConfig, RedactPairRule, Redaction, RuleSpec, RuleType};
use crate::processor::{SelectorPathItem, SelectorSpec};

pub fn to_pii_config(datascrubbing_config: &DataScrubbingConfig) -> Option<PiiConfig> {
    let mut custom_rules = BTreeMap::new();
    let mut applied_rules = Vec::new();

    if datascrubbing_config.scrub_data && datascrubbing_config.scrub_defaults {
        applied_rules.push("@common".to_owned());
    } else if datascrubbing_config.scrub_ip_addresses {
        applied_rules.push("@ip".to_owned());
    }

    if datascrubbing_config.scrub_data {
        let sensitive_fields_re = datascrubbing_config
            .sensitive_fields
            .iter()
            .filter(|x| !x.is_empty())
            .map(|x| regex::escape(&x))
            .join("|");

        if !sensitive_fields_re.is_empty() {
            custom_rules.insert(
                "strip-fields".to_owned(),
                RuleSpec {
                    ty: RuleType::RedactPair(RedactPairRule {
                        key_pattern: Pattern(
                            RegexBuilder::new(&format!(r".*({}).*", sensitive_fields_re))
                                .case_insensitive(true)
                                .build()
                                .unwrap(),
                        ),
                    }),
                    redaction: Redaction::Replace("[filtered: sensitive keys]".to_owned().into()),
                },
            );

            applied_rules.push("strip-fields".to_owned());
        }
    }

    if applied_rules.is_empty() {
        return None;
    }

    let mut selector = SelectorSpec::Path(vec![SelectorPathItem::DeepWildcard]);

    for field in &datascrubbing_config.exclude_fields {
        selector = SelectorSpec::And(
            Box::new(selector),
            Box::new(SelectorSpec::Not(Box::new(SelectorSpec::Path(vec![
                SelectorPathItem::Key(field.clone()),
            ])))),
        );
    }

    let mut applications = BTreeMap::new();
    applications.insert(selector, applied_rules.clone());

    Some(PiiConfig {
        rules: custom_rules,
        vars: Default::default(),
        applications,
    })
}

#[cfg(test)]
mod tests {
    use crate::datascrubbing::DataScrubbingConfig;
    /// These tests are ported from Sentry's Python testsuite (test_data_scrubber). Each testcase
    /// has an equivalent testcase in Python.
    use crate::pii::PiiProcessor;
    use crate::processor::{process_value, ProcessingState};
    use crate::protocol::Event;
    use crate::types::FromValue;

    use super::*;

    lazy_static::lazy_static! {
        static ref SENSITIVE_VARS: serde_json::Value = serde_json::json!({
            "foo": "bar",
            "password": "hello",
            "the_secret": "hello",
            "a_password_here": "hello",
            "api_key": "secret_key",
            "apiKey": "secret_key",
        });
    }

    static PUBLIC_KEY: &'static str = r#"""-----BEGIN PUBLIC KEY-----
MIICIjANBgkqhkiG9w0BAQEFAAOCAg8AMIICCgKCAgEA6A6TQjlPyMurLh/igZY4
izA9sJgeZ7s5+nGydO4AI9k33gcy2DObZuadWRMnDwc3uH/qoAPw/mo3KOcgEtxU
xdwiQeATa3HVPcQDCQiKm8xIG2Ny0oUbR0IFNvClvx7RWnPEMk05CuvsL0AA3eH5
xn02Yg0JTLgZEtUT3whwFm8CAwEAAQ==
-----END PUBLIC KEY-----"""#;

    static PRIVATE_KEY: &'static str = r#"""-----BEGIN PRIVATE KEY-----
MIIJRAIBADANBgkqhkiG9w0BAQEFAASCCS4wggkqAgEAAoICAQCoNFY4P+EeIXl0
mLpO+i8uFqAaEFQ8ZX2VVpA13kNEHuiWXC3HPlQ+7G+O3XmAsO+Wf/xY6pCSeQ8h
mLpO+i8uFqAaEFQ8ZX2VVpA13kNEHuiWXC3HPlQ+7G+O3XmAsO+Wf/xY6pCSeQ8h
-----END PRIVATE KEY-----"""#;

    static ENCRYPTED_PRIVATE_KEY: &'static str = r#"""-----BEGIN ENCRYPTED PRIVATE KEY-----
MIIJjjBABgkqhkiG9w0BBQ0wMzAbBgkqhkiG9w0BBQwwDgQIWVhErdQOFVoCAggA
IrlYQUV1ig4U3viYh1Y8viVvRlANKICvgj4faYNH36UterkfDjzMonb/cXNeJEOS
YgorM2Pfuec5vtPRPKd88+Ds/ktIlZhjJwnJjHQMX+lSw5t0/juna2sLH2dpuAbi
PSk=
-----END ENCRYPTED PRIVATE KEY-----"""#;

    static RSA_PRIVATE_KEY: &'static str = r#"""-----BEGIN RSA PRIVATE KEY-----
+wn9Iu+zgamKDUu22xc45F2gdwM04rTITlZgjAs6U1zcvOzGxk8mWJD5MqFWwAtF
zN87YGV0VMTG6ehxnkI4Fg6i0JPU3QIDAQABAoICAQCoCPjlYrODRU+vd2YeU/gM
THd+9FBxiHLGXNKhG/FRSyREXEt+NyYIf/0cyByc9tNksat794ddUqnLOg0vwSkv
-----END RSA PRIVATE KEY-----"""#;

    fn get_default_pii_config() -> PiiConfig {
        let pii_config = to_pii_config(&Default::default());

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {},
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        pii_config.unwrap()
    }

    #[test]
    fn test_stacktrace() {
        let mut data = Event::from_value(
            serde_json::json!({
                "stacktrace": {
                    "frames": [
                    {
                        "vars": SENSITIVE_VARS.clone()
                    }
                    ]
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "stacktrace": {
       ⋮    "frames": [
       ⋮      {
       ⋮        "vars": {
       ⋮          "a_password_here": null,
       ⋮          "apiKey": null,
       ⋮          "api_key": null,
       ⋮          "foo": "bar",
       ⋮          "password": null,
       ⋮          "the_secret": null
       ⋮        }
       ⋮      }
       ⋮    ]
       ⋮  },
       ⋮  "_meta": {
       ⋮    "stacktrace": {
       ⋮      "frames": {
       ⋮        "0": {
       ⋮          "vars": {
       ⋮            "a_password_here": {
       ⋮              "": {
       ⋮                "rem": [
       ⋮                  [
       ⋮                    "@password",
       ⋮                    "x"
       ⋮                  ]
       ⋮                ]
       ⋮              }
       ⋮            },
       ⋮            "apiKey": {
       ⋮              "": {
       ⋮                "rem": [
       ⋮                  [
       ⋮                    "@password",
       ⋮                    "x"
       ⋮                  ]
       ⋮                ]
       ⋮              }
       ⋮            },
       ⋮            "api_key": {
       ⋮              "": {
       ⋮                "rem": [
       ⋮                  [
       ⋮                    "@password",
       ⋮                    "x"
       ⋮                  ]
       ⋮                ]
       ⋮              }
       ⋮            },
       ⋮            "password": {
       ⋮              "": {
       ⋮                "rem": [
       ⋮                  [
       ⋮                    "@password",
       ⋮                    "x"
       ⋮                  ]
       ⋮                ]
       ⋮              }
       ⋮            },
       ⋮            "the_secret": {
       ⋮              "": {
       ⋮                "rem": [
       ⋮                  [
       ⋮                    "@password",
       ⋮                    "x"
       ⋮                  ]
       ⋮                ]
       ⋮              }
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_http() {
        let mut data = Event::from_value(
            serde_json::json!({
                "request": {
                    "data": SENSITIVE_VARS.clone(),
                    "env": SENSITIVE_VARS.clone(),
                    "headers": SENSITIVE_VARS.clone(),
                    "cookies": SENSITIVE_VARS.clone()
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "request": {
       ⋮    "data": {
       ⋮      "a_password_here": null,
       ⋮      "apiKey": null,
       ⋮      "api_key": null,
       ⋮      "foo": "bar",
       ⋮      "password": null,
       ⋮      "the_secret": null
       ⋮    },
       ⋮    "cookies": [
       ⋮      [
       ⋮        "a_password_here",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "apiKey",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "api_key",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "foo",
       ⋮        "bar"
       ⋮      ],
       ⋮      [
       ⋮        "password",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "the_secret",
       ⋮        null
       ⋮      ]
       ⋮    ],
       ⋮    "headers": [
       ⋮      [
       ⋮        "A_password_here",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "ApiKey",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "Api_key",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "Foo",
       ⋮        "bar"
       ⋮      ],
       ⋮      [
       ⋮        "Password",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "The_secret",
       ⋮        null
       ⋮      ]
       ⋮    ],
       ⋮    "env": {
       ⋮      "a_password_here": null,
       ⋮      "apiKey": null,
       ⋮      "api_key": null,
       ⋮      "foo": "bar",
       ⋮      "password": null,
       ⋮      "the_secret": null
       ⋮    }
       ⋮  },
       ⋮  "_meta": {
       ⋮    "request": {
       ⋮      "cookies": {
       ⋮        "0": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "1": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "2": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "4": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "5": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      },
       ⋮      "data": {
       ⋮        "a_password_here": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "apiKey": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "api_key": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "password": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "the_secret": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        }
       ⋮      },
       ⋮      "env": {
       ⋮        "a_password_here": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "apiKey": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "api_key": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "password": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "the_secret": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        }
       ⋮      },
       ⋮      "headers": {
       ⋮        "0": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "1": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "2": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "4": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "5": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_user() {
        let mut data = Event::from_value(
            serde_json::json!({
                "user": {
                    "username": "secret",
                    "data": SENSITIVE_VARS.clone()
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "user": {
       ⋮    "username": "",
       ⋮    "data": {
       ⋮      "a_password_here": null,
       ⋮      "apiKey": null,
       ⋮      "api_key": null,
       ⋮      "foo": "bar",
       ⋮      "password": null,
       ⋮      "the_secret": null
       ⋮    }
       ⋮  },
       ⋮  "_meta": {
       ⋮    "user": {
       ⋮      "data": {
       ⋮        "a_password_here": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "apiKey": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "api_key": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "password": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "the_secret": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        }
       ⋮      },
       ⋮      "username": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x",
       ⋮              0,
       ⋮              0
       ⋮            ]
       ⋮          ],
       ⋮          "len": 6
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_extra() {
        let mut data =
            Event::from_value(serde_json::json!({ "extra": SENSITIVE_VARS.clone() }).into());

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "a_password_here": null,
       ⋮    "apiKey": null,
       ⋮    "api_key": null,
       ⋮    "foo": "bar",
       ⋮    "password": null,
       ⋮    "the_secret": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "a_password_here": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "apiKey": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "api_key": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "password": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "the_secret": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_contexts() {
        let mut data = Event::from_value(
            serde_json::json!({
                "contexts": {
                    "secret": SENSITIVE_VARS.clone(),
                    "biz": SENSITIVE_VARS.clone()
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        // n.b.: This diverges from Python behavior because it would strip a context that is called
        // "secret", not just a string. We accept this difference.

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "contexts": {
       ⋮    "biz": {
       ⋮      "a_password_here": null,
       ⋮      "apiKey": null,
       ⋮      "api_key": null,
       ⋮      "foo": "bar",
       ⋮      "password": null,
       ⋮      "the_secret": null,
       ⋮      "type": "biz"
       ⋮    },
       ⋮    "secret": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "contexts": {
       ⋮      "biz": {
       ⋮        "a_password_here": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "apiKey": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "api_key": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "password": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        },
       ⋮        "the_secret": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        }
       ⋮      },
       ⋮      "secret": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_querystring_as_string() {
        let mut data = Event::from_value(serde_json::json!({
            "request": {
                "query_string": "foo=bar&password=hello&the_secret=hello&a_password_here=hello&api_key=secret_key",
            }
        }).into());

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        // n.b.: Python's datascrubbers return the query string as string again, while Rust parses
        // it during deserialization. In either case the PII is gone.

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "request": {
       ⋮    "query_string": [
       ⋮      [
       ⋮        "foo",
       ⋮        "bar"
       ⋮      ],
       ⋮      [
       ⋮        "password",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "the_secret",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "a_password_here",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "api_key",
       ⋮        null
       ⋮      ]
       ⋮    ]
       ⋮  },
       ⋮  "_meta": {
       ⋮    "request": {
       ⋮      "query_string": {
       ⋮        "1": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "2": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "3": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "4": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_querystring_as_pairlist() {
        let mut data = Event::from_value(
            serde_json::json!({
                "request": {
                    "query_string": [
                        ["foo", "bar"],
                        ["password", "hello"],
                        ["the_secret", "hello"],
                        ["a_password_here", "hello"],
                        ["api_key", "secret_key"]
                    ]
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "request": {
       ⋮    "query_string": [
       ⋮      [
       ⋮        "foo",
       ⋮        "bar"
       ⋮      ],
       ⋮      [
       ⋮        "password",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "the_secret",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "a_password_here",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "api_key",
       ⋮        null
       ⋮      ]
       ⋮    ]
       ⋮  },
       ⋮  "_meta": {
       ⋮    "request": {
       ⋮      "query_string": {
       ⋮        "1": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "2": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "3": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        },
       ⋮        "4": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_querystring_as_string_with_partials() {
        let mut data = Event::from_value(
            serde_json::json!({
                "request": {
                    "query_string": "foo=bar&password&baz=bar"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        // n.b.: Python's datascrubbers return the query string as string again, while Rust parses
        // it during deserialization. In either case the PII is gone.

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "request": {
       ⋮    "query_string": [
       ⋮      [
       ⋮        "foo",
       ⋮        "bar"
       ⋮      ],
       ⋮      [
       ⋮        "password",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "baz",
       ⋮        "bar"
       ⋮      ]
       ⋮    ]
       ⋮  },
       ⋮  "_meta": {
       ⋮    "request": {
       ⋮      "query_string": {
       ⋮        "1": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_querystring_as_pairlist_with_partials() {
        let mut data = Event::from_value(
            serde_json::json!({
                "request": {
                    "query_string": [["foo", "bar"], ["password", ""], ["baz", "bar"]]
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "request": {
       ⋮    "query_string": [
       ⋮      [
       ⋮        "foo",
       ⋮        "bar"
       ⋮      ],
       ⋮      [
       ⋮        "password",
       ⋮        null
       ⋮      ],
       ⋮      [
       ⋮        "baz",
       ⋮        "bar"
       ⋮      ]
       ⋮    ]
       ⋮  },
       ⋮  "_meta": {
       ⋮    "request": {
       ⋮      "query_string": {
       ⋮        "1": {
       ⋮          "1": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "@password",
       ⋮                  "x"
       ⋮                ]
       ⋮              ]
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_additional_sensitive_fields() {
        let mut extra = SENSITIVE_VARS.clone();
        {
            let map = extra.as_object_mut().unwrap();
            map.insert("fieldy_field".to_owned(), serde_json::json!("value"));
            map.insert(
                "moar_other_field".to_owned(),
                serde_json::json!("another value"),
            );
        }

        let mut data = Event::from_value(serde_json::json!({ "extra": extra }).into());

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["fieldy_field".to_owned(), "moar_other_field".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {
       ⋮    "strip-fields": {
       ⋮      "type": "redact_pair",
       ⋮      "keyPattern": ".*(fieldy_field|moar_other_field).*",
       ⋮      "redaction": {
       ⋮        "method": "replace",
       ⋮        "text": "[filtered: sensitive keys]"
       ⋮      }
       ⋮    }
       ⋮  },
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common",
       ⋮      "strip-fields"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "a_password_here": null,
       ⋮    "apiKey": null,
       ⋮    "api_key": null,
       ⋮    "fieldy_field": null,
       ⋮    "foo": "bar",
       ⋮    "moar_other_field": null,
       ⋮    "password": null,
       ⋮    "the_secret": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "a_password_here": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "apiKey": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "api_key": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "fieldy_field": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "strip-fields",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "moar_other_field": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "strip-fields",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "password": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "the_secret": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_credit_card() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "4571234567890111"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "[creditcard]"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              0,
       ⋮              12
       ⋮            ]
       ⋮          ],
       ⋮          "len": 16
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_credit_card_amex() {
        // AMEX numbers are 15 digits, not 16
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "378282246310005"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "[creditcard]"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              0,
       ⋮              12
       ⋮            ]
       ⋮          ],
       ⋮          "len": 15
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_credit_card_discover() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "6011111111111117"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "[creditcard]"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              0,
       ⋮              12
       ⋮            ]
       ⋮          ],
       ⋮          "len": 16
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_credit_card_visa() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "4111111111111111"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "[creditcard]"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              0,
       ⋮              12
       ⋮            ]
       ⋮          ],
       ⋮          "len": 16
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_credit_card_mastercard() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "5555555555554444"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "[creditcard]"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              0,
       ⋮              12
       ⋮            ]
       ⋮          ],
       ⋮          "len": 16
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_credit_card_within_value() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "'4571234567890111'"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "'[creditcard]'"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              1,
       ⋮              13
       ⋮            ]
       ⋮          ],
       ⋮          "len": 18
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_credit_card_within_value2() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "foo 4571234567890111"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "foo [creditcard]"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              4,
       ⋮              16
       ⋮            ]
       ⋮          ],
       ⋮          "len": 20
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_does_not_sanitize_timestamp_looks_like_card() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "1453843029218310"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "1453843029218310"
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_url1() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "pg://matt:pass@localhost/1"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "pg://matt:[email]/1"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              10,
       ⋮              17
       ⋮            ]
       ⋮          ],
       ⋮          "len": 26
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_url2() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "foo 'redis://redis:foo@localhost:6379/0' bar"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "foo 'redis://redis:[email]:6379/0' bar"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              19,
       ⋮              26
       ⋮            ]
       ⋮          ],
       ⋮          "len": 44
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_url3() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "'redis://redis:foo@localhost:6379/0'"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "'redis://redis:[email]:6379/0'"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              15,
       ⋮              22
       ⋮            ]
       ⋮          ],
       ⋮          "len": 36
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_url4() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "foo redis://redis:foo@localhost:6379/0 bar"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "foo redis://redis:[email]:6379/0 bar"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              18,
       ⋮              25
       ⋮            ]
       ⋮          ],
       ⋮          "len": 42
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_url5() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "foo redis://redis:foo@localhost:6379/0 bar pg://matt:foo@localhost/1"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "foo redis://redis:[email]:6379/0 bar pg://matt:[email]/1"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              18,
       ⋮              25
       ⋮            ],
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              47,
       ⋮              54
       ⋮            ]
       ⋮          ],
       ⋮          "len": 68
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_url6() {
        // Make sure we don't mess up any other url.
        // This url specifically if passed through urlunsplit(urlsplit()),
        // it'll change the value.
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": "postgres:///path"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "postgres:///path"
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_url7() {
        // Don't be too overly eager within JSON strings an catch the right field.
        // n.b.: We accept the difference from Python, where "b" is not masked.
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": r#"{"a":"https://localhost","b":"foo@localhost","c":"pg://matt:pass@localhost/1","d":"lol"}"#
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": "{\"a\":\"https://localhost\",\"b\":\"[email]\",\"c\":\"pg://matt:[email]/1\",\"d\":\"lol\"}"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "foo": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              30,
       ⋮              37
       ⋮            ],
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              54,
       ⋮              61
       ⋮            ]
       ⋮          ],
       ⋮          "len": 88
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_sanitize_http_body() {
        use crate::store::StoreProcessor;

        let mut data = Event::from_value(
            serde_json::json!({
                "request": {
                    "data": r#"{"email":"zzzz@gmail.com","password":"zzzzz"}"#
                }
            })
            .into(),
        );

        // n.b.: In Rust we rely on store normalization to parse inline JSON

        let mut store_processor = StoreProcessor::new(Default::default(), None);
        process_value(&mut data, &mut store_processor, ProcessingState::root());

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.value().unwrap().request.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "data": {
       ⋮    "email": "[email]",
       ⋮    "password": null
       ⋮  },
       ⋮  "inferred_content_type": "application/json",
       ⋮  "_meta": {
       ⋮    "data": {
       ⋮      "email": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@email",
       ⋮              "s",
       ⋮              0,
       ⋮              7
       ⋮            ]
       ⋮          ],
       ⋮          "len": 14
       ⋮        }
       ⋮      },
       ⋮      "password": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_does_not_fail_on_non_string() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "foo": 1
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foo": 1
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_does_sanitize_public_key() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "s": PUBLIC_KEY,
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "s": "\"\"-----BEGIN PUBLIC KEY-----\n[pemkey]\n-----END PUBLIC KEY-----\"\""
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "s": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@pemkey",
       ⋮              "s",
       ⋮              29,
       ⋮              37
       ⋮            ]
       ⋮          ],
       ⋮          "len": 283
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_does_sanitize_private_key() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "s": PRIVATE_KEY,
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "s": "\"\"-----BEGIN PRIVATE KEY-----\n[pemkey]\n-----END PRIVATE KEY-----\"\""
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "s": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@pemkey",
       ⋮              "s",
       ⋮              30,
       ⋮              38
       ⋮            ]
       ⋮          ],
       ⋮          "len": 252
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_does_sanitize_encrypted_private_key() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "s": ENCRYPTED_PRIVATE_KEY,
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "s": "\"\"-----BEGIN ENCRYPTED PRIVATE KEY-----\n[pemkey]\n-----END ENCRYPTED PRIVATE KEY-----\"\""
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "s": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@pemkey",
       ⋮              "s",
       ⋮              40,
       ⋮              48
       ⋮            ]
       ⋮          ],
       ⋮          "len": 277
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_does_sanitize_rsa_private_key() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "s": RSA_PRIVATE_KEY,
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "s": "\"\"-----BEGIN RSA PRIVATE KEY-----\n[pemkey]\n-----END RSA PRIVATE KEY-----\"\""
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "s": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@pemkey",
       ⋮              "s",
       ⋮              34,
       ⋮              42
       ⋮            ]
       ⋮          ],
       ⋮          "len": 260
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_does_sanitize_social_security_number() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "s": "123-45-6789"
                }
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "s": "***-**-****"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "s": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@usssn",
       ⋮              "m",
       ⋮              0,
       ⋮              11
       ⋮            ]
       ⋮          ],
       ⋮          "len": 11
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_exclude_fields_on_field_name() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {"password": "123-45-6789"}
            })
            .into(),
        );

        let pii_config = get_default_pii_config();
        let mut pii_processor = PiiProcessor::new(&pii_config);
        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "password": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "password": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_explicit_fields() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {"mystuff": "xxx"}
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["mystuff".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {
       ⋮    "strip-fields": {
       ⋮      "type": "redact_pair",
       ⋮      "keyPattern": ".*(mystuff).*",
       ⋮      "redaction": {
       ⋮        "method": "replace",
       ⋮        "text": "[filtered: sensitive keys]"
       ⋮      }
       ⋮    }
       ⋮  },
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common",
       ⋮      "strip-fields"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "mystuff": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "mystuff": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "strip-fields",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_explicit_fields_case_insensitive() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {"MYSTUFF": "xxx"}
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["myStuff".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {
       ⋮    "strip-fields": {
       ⋮      "type": "redact_pair",
       ⋮      "keyPattern": ".*(myStuff).*",
       ⋮      "redaction": {
       ⋮        "method": "replace",
       ⋮        "text": "[filtered: sensitive keys]"
       ⋮      }
       ⋮    }
       ⋮  },
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common",
       ⋮      "strip-fields"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "MYSTUFF": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "MYSTUFF": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "strip-fields",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_exclude_fields_on_field_value() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {"foobar": "123-45-6789"}
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            exclude_fields: vec!["foobar".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {},
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "(**&~foobar)": [
       ⋮      "@common"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foobar": "123-45-6789"
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_empty_field() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {"foobar": "xxx"}
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {},
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "foobar": "xxx"
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_should_have_mysql_pwd_as_a_default() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "MYSQL_PWD": "the one",
                    "mysql_pwd": "the two",
                }
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {},
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "MYSQL_PWD": null,
       ⋮    "mysql_pwd": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "MYSQL_PWD": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "mysql_pwd": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_authorization_scrubbing() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "authorization": "foobar",
                    "auth": "foobar",
                    "auXth": "foobar",
                }
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {},
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "auXth": "foobar",
       ⋮    "auth": null,
       ⋮    "authorization": null
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "auth": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      },
       ⋮      "authorization": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@password",
       ⋮              "x"
       ⋮            ]
       ⋮          ]
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_doesnt_scrub_not_scrubbed() {
        let mut data = Event::from_value(
            serde_json::json!({
                "extra": {
                    "test1": {
                        "is_authenticated": "foobar",
                    },

                    "test2": {
                        "is_authenticated": "null",
                    },

                    "test3": {
                        "is_authenticated": true,
                    },
                }
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {},
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "extra": {
       ⋮    "test1": {
       ⋮      "is_authenticated": null
       ⋮    },
       ⋮    "test2": {
       ⋮      "is_authenticated": "null"
       ⋮    },
       ⋮    "test3": {
       ⋮      "is_authenticated": true
       ⋮    }
       ⋮  },
       ⋮  "_meta": {
       ⋮    "extra": {
       ⋮      "test1": {
       ⋮        "is_authenticated": {
       ⋮          "": {
       ⋮            "rem": [
       ⋮              [
       ⋮                "@password",
       ⋮                "x"
       ⋮              ]
       ⋮            ]
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_csp_blocked_uri() {
        let mut data = Event::from_value(
            serde_json::json!({
                "csp": {"blocked_uri": "https://example.com/?foo=4571234567890111&bar=baz"}
            })
            .into(),
        );

        let pii_config = get_default_pii_config();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "csp": {
       ⋮    "blocked_uri": "https://example.com/?foo=[creditcard]&bar=baz"
       ⋮  },
       ⋮  "_meta": {
       ⋮    "csp": {
       ⋮      "blocked_uri": {
       ⋮        "": {
       ⋮          "rem": [
       ⋮            [
       ⋮              "@creditcard:replace",
       ⋮              "s",
       ⋮              25,
       ⋮              37
       ⋮            ]
       ⋮          ],
       ⋮          "len": 49
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }

    #[test]
    fn test_breadcrumb_message() {
        let mut data = Event::from_value(
            serde_json::json!({
                "breadcrumbs": {
                    "values": [
                        {
                            "message": "SELECT session_key FROM django_session WHERE session_key = 'abcdefg'"
                        }
                    ]
                }
            })
            .into(),
        );

        let pii_config = to_pii_config(&DataScrubbingConfig {
            sensitive_fields: vec!["session_key".to_owned()],
            ..Default::default()
        });

        insta::assert_json_snapshot_matches!(pii_config, @r###"
       ⋮{
       ⋮  "rules": {
       ⋮    "strip-fields": {
       ⋮      "type": "redact_pair",
       ⋮      "keyPattern": ".*(session_key).*",
       ⋮      "redaction": {
       ⋮        "method": "replace",
       ⋮        "text": "[filtered: sensitive keys]"
       ⋮      }
       ⋮    }
       ⋮  },
       ⋮  "vars": {
       ⋮    "hashKey": null
       ⋮  },
       ⋮  "applications": {
       ⋮    "**": [
       ⋮      "@common",
       ⋮      "strip-fields"
       ⋮    ]
       ⋮  }
       ⋮}
        "###);

        let pii_config = pii_config.unwrap();

        let mut pii_processor = PiiProcessor::new(&pii_config);

        process_value(&mut data, &mut pii_processor, ProcessingState::root());

        insta::assert_snapshot_matches!(data.to_json_pretty().unwrap(), @r###"
       ⋮{
       ⋮  "breadcrumbs": {
       ⋮    "values": [
       ⋮      {
       ⋮        "message": "[filtered: sensitive keys]"
       ⋮      }
       ⋮    ]
       ⋮  },
       ⋮  "_meta": {
       ⋮    "breadcrumbs": {
       ⋮      "values": {
       ⋮        "0": {
       ⋮          "message": {
       ⋮            "": {
       ⋮              "rem": [
       ⋮                [
       ⋮                  "strip-fields",
       ⋮                  "s",
       ⋮                  0,
       ⋮                  26
       ⋮                ]
       ⋮              ],
       ⋮              "len": 68
       ⋮            }
       ⋮          }
       ⋮        }
       ⋮      }
       ⋮    }
       ⋮  }
       ⋮}
        "###);
    }
}

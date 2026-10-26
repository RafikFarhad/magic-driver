use crate::io::uinput::TOUCHPAD_IDENTITY;

const RULES: &str = include_str!("../../../dist/70-magic-driver.rules");

#[derive(Debug, Clone, PartialEq, Eq)]
struct Assignment {
    key: String,
    operator: String,
    value: String,
}

fn parse_rule(line: &str) -> Vec<Assignment> {
    let mut assignments = Vec::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        let operator_start = rest
            .find(['=', '!', '+', '-', ':'])
            .unwrap_or_else(|| panic!("no operator in {rest:?}"));
        let key = rest[..operator_start].trim().to_owned();
        let after_key = &rest[operator_start..];
        let operator_len = if after_key.starts_with("==")
            || after_key.starts_with("!=")
            || after_key.starts_with("+=")
            || after_key.starts_with("-=")
            || after_key.starts_with(":=")
        {
            2
        } else {
            1
        };
        let operator = after_key[..operator_len].to_owned();
        let after_operator = after_key[operator_len..].trim_start();
        assert!(
            after_operator.starts_with('"'),
            "unquoted value in {line:?}"
        );
        let close = after_operator[1..]
            .find('"')
            .unwrap_or_else(|| panic!("unterminated value in {line:?}"));
        let value = after_operator[1..=close].to_owned();
        assignments.push(Assignment {
            key,
            operator,
            value,
        });
        rest = after_operator[close + 2..].trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
    assignments
}

fn rules() -> Vec<Vec<Assignment>> {
    RULES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(parse_rule)
        .collect()
}

fn get<'a>(rule: &'a [Assignment], key: &str) -> Option<&'a Assignment> {
    rule.iter().find(|assignment| assignment.key == key)
}

fn matches(rule: &[Assignment], key: &str, value: &str) -> bool {
    get(rule, key)
        .is_some_and(|assignment| assignment.operator == "==" && assignment.value == value)
}

fn tags_uaccess(rule: &[Assignment]) -> bool {
    rule.iter().any(|assignment| {
        assignment.key == "TAG" && assignment.operator == "+=" && assignment.value == "uaccess"
    })
}

#[test]
fn rule_file_parses() {
    let rules = rules();
    assert_eq!(rules.len(), 6);
    for rule in &rules {
        assert!(!rule.is_empty());
    }
}

#[test]
fn no_mode_group_or_owner_keys() {
    for rule in rules() {
        for assignment in rule {
            assert!(
                !matches!(assignment.key.as_str(), "MODE" | "GROUP" | "OWNER"),
                "forbidden key {assignment:?}"
            );
        }
    }
    assert!(!RULES.contains("MODE"));
    assert!(!RULES.contains("GROUP"));
}

#[test]
fn exactly_three_uaccess_lines() {
    let rules = rules();
    let uaccess: Vec<_> = rules.iter().filter(|rule| tags_uaccess(rule)).collect();
    assert_eq!(uaccess.len(), 3);
    assert!(uaccess.iter().any(|rule| {
        matches(rule, "SUBSYSTEM", "hidraw") && matches(rule, "KERNELS", "0005:004C:0269.*")
    }));
    assert!(uaccess.iter().any(|rule| {
        matches(rule, "SUBSYSTEM", "input")
            && matches(rule, "KERNEL", "event*")
            && matches(rule, "KERNELS", "0005:004C:0269.*")
    }));
    assert!(
        uaccess.iter().any(|rule| {
            matches(rule, "SUBSYSTEM", "misc") && matches(rule, "KERNEL", "uinput")
        })
    );
}

#[test]
fn uaccess_lines_are_limited_to_the_magic_mouse_and_uinput() {
    for rule in rules().iter().filter(|rule| tags_uaccess(rule)) {
        let limited =
            matches(rule, "KERNELS", "0005:004C:0269.*") || matches(rule, "KERNEL", "uinput");
        assert!(limited, "{rule:?}");
    }
}

#[test]
fn virtual_touchpad_is_marked_external_by_name_and_ids() {
    let rules = rules();
    let rule = rules
        .iter()
        .find(|rule| get(rule, "ENV{ID_INPUT_TOUCHPAD_INTEGRATION}").is_some())
        .expect("integration rule");
    let integration = get(rule, "ENV{ID_INPUT_TOUCHPAD_INTEGRATION}").unwrap();
    assert_eq!(integration.operator, "=");
    assert_eq!(integration.value, "external");
    assert!(matches(rule, "SUBSYSTEM", "input"));
    assert!(matches(rule, "KERNEL", "event*"));
    assert!(matches(rule, "ATTRS{name}", TOUCHPAD_IDENTITY.name));
    assert!(matches(
        rule,
        "ATTRS{id/bustype}",
        &format!("{:04x}", TOUCHPAD_IDENTITY.bus)
    ));
    assert!(matches(
        rule,
        "ATTRS{id/vendor}",
        &format!("{:04x}", TOUCHPAD_IDENTITY.vendor)
    ));
    assert!(matches(
        rule,
        "ATTRS{id/product}",
        &format!("{:04x}", TOUCHPAD_IDENTITY.product)
    ));
    assert!(!tags_uaccess(rule));
}

#[test]
fn remove_events_skip_the_rules() {
    let rules = rules();
    assert!(matches(&rules[0], "ACTION", "remove"));
    assert_eq!(get(&rules[0], "GOTO").unwrap().value, "magic_driver_end");
    assert_eq!(
        get(rules.last().unwrap(), "LABEL").unwrap().value,
        "magic_driver_end"
    );
}

#[test]
fn parser_handles_operators() {
    let rule = parse_rule(r#"KERNEL=="a", TAG+="b", ENV{X}="c", ATTR{y}!="d""#);
    let operators: Vec<_> = rule.iter().map(|a| a.operator.as_str()).collect();
    assert_eq!(operators, vec!["==", "+=", "=", "!="]);
    assert_eq!(rule[2].key, "ENV{X}");
}

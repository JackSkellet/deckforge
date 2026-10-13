#[path = "../src/action_wheel.rs"]
pub mod action_wheel;
use action_wheel::*;
use serde_json::json;
use std::time::Duration;
fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn profiles_keep_unknown_data_and_normalize_native_selection() {
    let mut raw = json!({"UUID":UUID,"Settings":{"CurrentIdx":1,"ControlOffset":0,"ExtTitles":true,"InvertRotation":true,"Future":73},"Actions":[{"UUID":"test.a","ActionID":"a","Settings":null,"FutureChild":true},{"UUID":"test.b","ActionID":"b","States":[{}]}],"FutureParent":[1,2]});
    let wheel = parse(&raw).unwrap();
    assert_eq!(wheel.current, 1);
    assert!(wheel.extended_titles && wheel.invert_rotation);
    assert_eq!(wheel.raw, raw);
    assert_eq!(wheel.children[0].raw["Settings"], serde_json::Value::Null);
    raw["Settings"]["CurrentIdx"] = json!(900);
    assert_eq!(parse(&raw).unwrap().current, 1);
    raw["Settings"]["CurrentIdx"] = json!(-1);
    assert_eq!(parse(&raw).unwrap().current, 1);
    raw["Settings"]["CurrentIdx"] = json!(-0.9);
    assert_eq!(parse(&raw).unwrap().current, 0);
    raw["Settings"]["CurrentIdx"] = json!(0.9);
    assert_eq!(parse(&raw).unwrap().current, 0);
    raw["Actions"][1]["ActionID"] = json!("a");
    assert!(parse(&raw).is_err());
    assert!(parse(&json!({"Actions":[{"UUID":UUID}]})).is_err());
    assert!(parse(&json!({"Actions":[],"Settings":{"ExtTitles":"true"}})).is_err());
    assert_eq!(parse(&json!({"Actions":[]})).unwrap().current, 0);
}
#[test]
fn turns_are_capped_replaced_and_consumed_on_the_native_cadence() {
    let mut rotation = Rotation::default();
    assert!(rotation.request(127, false, false, 10, ms(0)));
    assert_eq!(rotation.advance(0, 10, ms(0)), Some(4));
    assert_eq!(rotation.next_due(), Some(ms(61)));
    assert_eq!(rotation.advance(4, 10, ms(60)), None);
    assert_eq!(rotation.advance(4, 10, ms(61)), Some(5));
    assert!(rotation.request(-2, false, false, 10, ms(62)));
    assert_eq!(rotation.advance(5, 10, ms(62)), None);
    assert!(!rotation.request(5, true, false, 10, ms(80)));
    assert_eq!(rotation.advance(5, 10, ms(122)), Some(3));
    assert!(rotation.request(-128, false, true, 10, ms(123)));
    assert_eq!(rotation.advance(3, 10, ms(183)), Some(7));
    assert!(rotation.request(-1, false, false, 10, ms(184)));
    assert_eq!(rotation.advance(7, 10, ms(244)), Some(6));
    assert!(rotation.request(0, false, false, 10, ms(245)));
    assert_eq!(rotation.advance(6, 10, ms(305)), None);
    assert_eq!(rotation.next_due(), None);
}
#[test]
fn one_two_and_many_item_hit_regions_follow_front_card_priority() {
    assert!(cards(0, 0).is_empty());
    assert_eq!(hit(1, 0, 80, 30), Some(Hit::Activate));
    assert_eq!(hit(1, 0, 25, 30), None);
    assert_eq!(hit(2, 0, 105, 30), Some(Hit::Activate));
    assert_eq!(hit(2, 0, 140, 30), Some(Hit::Next));
    assert_eq!(hit(2, 1, 90, 30), Some(Hit::Activate));
    assert_eq!(hit(2, 1, 50, 30), Some(Hit::Previous));
    let items = cards(6, 0);
    assert_eq!(items.iter().map(|c| c.index).collect::<Vec<_>>(), [5, 1, 0]);
    assert_eq!(hit(6, 0, 70, 30), Some(Hit::Activate));
    assert_eq!(hit(6, 0, 25, 30), Some(Hit::Previous));
    assert_eq!(hit(6, 0, 150, 30), Some(Hit::Next));
    assert_eq!(hit(6, 0, 150, 84), None);
}
#[test]
fn indicator_viewport_keeps_selection_visible_with_nine_items_at_most() {
    for count in 0..=MAX_CHILDREN {
        let mut bar = Indicators::new(count, 0, usize::MAX);
        for current in (0..count).chain((0..count).rev()) {
            bar.select(current);
            let items = bar.items();
            assert!(items.len() <= 9);
            assert_eq!(items.iter().filter(|i| i.active).count(), 1);
            assert!(items.iter().any(|i| i.index == Some(current) && i.active));
            assert!(bar.offset() < count);
        }
    }
    let items = Indicators::new(6, 1, 0).items();
    assert_eq!(items.len(), 6);
    assert_eq!((items[1].width, items[1].height), (12, 8));
}

#[test]
#[ignore = "requires the retained original Stream Deck 7.6 profile"]
fn original_six_clock_wheel_parses_without_changing_its_records() {
    use sha2::Digest;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../evidence/elgato-7.6/raw/flsLUxNAo3HxRBo8yay7EbNtFyJC9c");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(&bytes)),
        "e4ce1ac8947e28266b852a0860c03c9437b7ed2a2e2bd5044e1b94a502a72675"
    );
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let member = "Profiles/D96C93D6-FC9E-4EC6-BC43-AC3DDB5A2F39.sdProfile/Profiles/6B53D0C1-DD39-4FBA-A38E-87021FEDEC39/manifest.json";
    let page: serde_json::Value =
        serde_json::from_reader(archive.by_name(member).unwrap()).unwrap();
    let raw = &page["Controllers"][0]["Actions"]["1,0"];
    let wheel = parse(raw).unwrap();
    assert_eq!(wheel.children.len(), 6);
    assert_eq!(wheel.current, 1);
    assert!(!wheel.extended_titles && !wheel.invert_rotation);
    assert_eq!(wheel.raw, *raw);
    for (child, original) in wheel
        .children
        .iter()
        .zip(raw["Actions"].as_array().unwrap())
    {
        assert_eq!(child.raw, *original);
    }
}

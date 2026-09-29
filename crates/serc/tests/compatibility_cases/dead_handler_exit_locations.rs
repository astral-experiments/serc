use crate::common;

const SOURCE: &str = r#"if __name__ == "__main__":
    import json

    record = json.load(open("input.json"))
    payload = json.dumps(record).encode("utf-8")
    with open("output.json", "wb") as stream:
        stream.write(payload)
    record = json.load(open("output.json"))[str(1)]
    print(record)
    assert False
    import random

    seeds = list(range(11))[1:]
    records = {}
    for seed in seeds:
        random.seed(seed)
        engine = Engine(size=random.randint(3, 7))
        state = engine.reset()
        done = False
        totals = [0.0 for _ in range(engine.size)]
        while not done:
            choices = engine.choices()
            if not choices:
                break
            state, reward, done, *rest = engine.step(random.choice(choices))
            totals[engine.current] += reward
        records[seed] = totals

    import json
    with open("records.json", "w") as stream:
        json.dump(records, stream, default=lambda value: value.tolist() if hasattr(value, "tolist") else value)
    engine.close()
"#;

#[test]
fn dead_handler_exit_locations_explicit_raise_control() {
    let source = SOURCE.replace("    assert False\n", "    raise AssertionError\n");
    common::assert_matches_cpython_marshal(
        &source,
        "module_with_dead_manager_exit_raise_control.py",
    );
}

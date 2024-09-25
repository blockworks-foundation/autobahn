use std::env;

pub fn string_or_env(value_or_env: String) -> String {
    let value = match &value_or_env.chars().next().unwrap() {
        '$' => env::var(&value_or_env[1..]).expect("reading from env"),
        _ => value_or_env,
    };

    value
}

pub fn tracing_subscriber_init() {
    let format = tracing_subscriber::fmt::format().with_ansi(atty::is(atty::Stream::Stdout));

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .event_format(format)
        .init();
}

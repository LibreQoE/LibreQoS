# LQosConfig

`lqos_config` is designed to manage configuration of LibreQoS. Starting in 1.5, all configuration is
centralized into `/etc/lqos.conf`.

The `lqos_python` module contains functions that mirror each of these, using their original Python
names for integration purposes.

You can find the full definitions of each configuration entry in `src/etc/v15`.

## Adding Configuration Items

There are two ways to add a configuration:

1. Declare a Major Version Break. This is a whole new setup that will require a new configuration and migration. We should avoid doing this very often.
    1. You need to create a new folder, e.g. `src/etc/v16`.
    2. You need to port as much of the old config as you are creating.
    3. You need to update `src/etc/migration.rs` to include code to read a "v15" file and create a "v16" configuration.
    4. *This is a lot of work and should be a planned effort!*
2. Declare an optional new version item. This is how you handle "oh, I needed to snoo the foo" - and add an *optional* configuration item - so nothing will snarl up because it isn't there.
    1. Find the section you want to include it in in `src/etc/v15`. If there isn't one, create it using one of the others as a template and be sure to include the defaults. Add it into `top_config` as the type `Option<MySnooFoo>`.
    2. Update `example.toml` to include what *should* go there.
    3. Go into `lqos_python` and in `lib.rs` add a Python "getter" for the field. Remember to use `if let` to read the `Option` and return a default if it isn't present.

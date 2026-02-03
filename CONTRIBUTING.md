> This is **draft 1** of the contributing guide. Comments/edits appreciated.

# Contributing to LibreQoS

So your interested in contributing to LibreQoS! Awesome! Feel free to pitch in with whatever interests you, and we'll be happy to help.

Check out our [Code of Conduct](https://github.com/LibreQoE/LibreQoS/blob/main/.github/CODE_OF_CONDUCT.md), we'd like to keep this a happy, constructive place. Also, please join the chat on [Matrix](https://app.element.io/#/room/#libreqos:matrix.org)---the core developers hang out there, and will be happy to help you.

In particular:

* We can check that an issue isn't already being worked on. There's nothing more frustrating than working hard on an issue only to discover that someone else already fixed it!
* We can help point you at issues that might interest you.
* We can offer help and/or mentoring with Rust and C.

# How You Can Help

There's lots of ways you can help:

* **Battle-Testing LibreQos**: use the software, let us know what does/doesn't work!
* **Donate**: it's free software, but we appreciate your support.
* **Let us Know Your Thoughts**: Hop on the chat or our discussions page and let us know what you're thinking about.
* **Finding Bugs** - something doesn't work for you? We can't fix it if you don't tell us about it.
* **Fixing Bugs** and **Adding Features**: See the "Development Guide", below.
* **Teaching Others**: if LibreQoS is working well for you, you can help others benefit by sharing your experience. See someone struggling? Feel free to pitch in and help them.

LibreQos strive to create an open, friendly environment. If you have an idea for how to help, let us know.

# Development Guidelines

This section contains some advice to help get you started writing code for LibreQoS.

## Getting Oriented with LibreQoS

LibreQos is divided into several sections:

* **Rust: system management/control plane**
   * **System Daemons**
      * `lqosd` is probably the most important part of the Rust system. It loads the eBPF system that provides bridging and traffic shaping control, runs the "bus" for other systems to communicate, and gathers information directly from the eBPF and kernel systems. It has several sub-crates:
         * `lqos_heimdall` handles all of the packet sniffing, flow tracking and `libcap` compatible packet data.
         * `lqos_queue_tracker` maintains statistics for Linux `tc` shaping queues, particularly Cake.
      * `lqos_node_manager` provides a per-node management web interface. It gathers most of its data from `lqosd` via the bus.
   * **CLI Utilities**
      * `lqusers` provides a CLI interface for managing authentication to the node manager.
      * `lqtop` provides a quick and easy way to see what the shaper is doing from the local management console.
      * `xdp_iphash_to_cpu_cmdline` (the name is inherited from previous projects) provides a command-line interface to map IP subnets to TC and CPU handles.
      * `xdp_pping` provides a CLI tool to give a quick summary of TCP RTT times, grouped by TC handle.
   * **Libraries**
      * `lqos_bus` provides a local-only (never leaving the shaper node) inter-process communication system. It's used by programs that need to ask `lqosd` to do something, or retrieve information from it. 
         * The `lqos_bus` crate also acts as a repository for shared data structures when data is passed between portions of the program.
      * `lqos_config` manages Rust integration with the `ispConfig.py` configuration file, and the `/etc/lqos.conf` file. It is designed as a helper for other systems to quickly access configuration parameters.
      * `lqos_python` compiles into a Python loadable library, providing a convenient interface to Rust code from the Python portions of the program.
      * `lqos_setup` provides a text-based initial setup system for users who install LibreQoS via `apt-get`.
      * `lqos_utils` provides a grab-bag of handy functions we've found useful elsewhere in the system.
* **Python: system operation and integration**
   * `LibreQoS.py` maps all circuits to TC handles and gets the shaper system running.
   * `ispConfig.py` provides a system-wide configuration.
   * `integrationX.py` provide integrations with UISP, Splynx and other CRM tools.
   * `lqTools.py` provides an interface for gathering statistics.

## What We're Building

LibreQoS is a free/open source fair queueing system. It's designed for ISPs, but can be useful elsewhere. Primary goals:

* Provide fair-queueing to help maximize the use of the Internet resources you have.
* Keep end-user latency low.
* Don't alter user traffic (for example by lowering the quality of their streaming video).
* Don't invade user's privacy.
* Provide excellent support tools to help keep your ISP running smoothly.

Some secondary goals include:

* Visualize data to facilitate moving the state-of-the-art in fair queueing forwards.
* Provide amazing throughput on inexpensive hardware.

## Making Changes to LibreQoS

We try to remain fast-moving and "process light".

> When we use the word "agile", we don't mean the heavily formalized process with scrums, kanban boards and similar. We mean lightweight, fast moving, and adhering to useful guides like "Code Complete" and "The Pragmatic Programmer". Not getting bogged down in heavy process.

### Simple Changes

For a straightforward change, do the following:

1. Do one (or more!) of the following:
   * Let us know on the chat that you're working on it.
   * Create an Issue in our GitHub repo.
   * Submit a PR, following the Branch Guidelines below.
2. Other community members review and comment on your change in an informal manner.
3. Once consensus is reached, we'll merge to the `develop` branch (or a branch parented from `develop` if it's a big change) and test it on our server resources at Equinix.
4. When ready, we'll merge it into the next release.

### Complex Changes

If you need/want a complicated change, please get in touch with us on the Matrix chat first. We'd like to avoid duplicating effort and wasting anyone's time. We'll be happy to offer advice and guidance. Then the process is similar:

1. Work on your local branch, parented off of `develop`.
2. Create a PR (targeting `develop`). If you'd like interim feedback, create a "draft PR" and we can help test your branch before the PR finalizes.
3. Once your PR is ready, submit it.
4. The community will review/comment on your PR.
5. Once consensus is reached, we'll merge it into `develop`.
6. Once ready, `develop` will merge into `main`.

## Branch Guidelines

LibreQoS has adopted the following scheme for branches:

* `main` - released code (tagged at releases), safe to pull.
   * `develop` - Parented from `main` and rebased on release. Nothing gets directly comitted into `develop`, it's the parent tree for ongoing development work.
      * `my_feature` - if you're working on a feature, your feature branch goes here - with `develop` as the parent. PRs---once the feature is ready for inclusion---should be targeted at `develop`.
      * `issue_xxx_name` - if you're working on a bugfix, work on it in a branch here. When the issue resolution is complete, PRs should be targeted at `develop`.
  * `hotfix_xxx` - If an emergency occurs and you have to push a fix to `main` in a hurry, hotfix branches can be parented from `main`. The resulting PR should target `main`. Let the developers know that they need to rebase `develop`.

The goal is for `main` to *always* be safe to clone and run, without surprises.

## Code Guidelines

This is very much a work in progress.

### Rust

#### Code Formatting

* Use `cargo fmt` to format your code. We've got a customized format setup in place.
* Adhere to standard Rust case and naming guidelines.

#### Dependencies

* Check that you aren't including any dependencies that incompatible with our license---GPL v2.
* Look through the `Cargo.toml` files (or run `cargo tree`) and try to prefer using a dependency we use elsewhere.
* Try to avoid using unmaintained crates.

#### Naming

* Don't use short, incomprehensible names in any API or function accessible from outside. You don't save any RAM by naming your variable `sz` instead of `size`---you just make it harder for anyone reading the code.
* It's fine to use `i` and similar for internal counting iterators. Try to use meaningful names for everything else.

#### Code Style

* Prefer functional/iterative code to imperative. Sometimes you need a `for` loop, but if it can be replaced with an `iter`, `map` and `fold` it will both compile into faster code and be less error prone.
* It's better to have lots of small functions than one big one. Rust is really good at inlining, and it's much easier to understand short functions. It's also easier to test small functions.
* If you have to override a Clippy warning, add a comment explaining why you did so.
* Functions accessible from other crates should use the RustDoc standard for documentation.

#### Unit Tests

* If you fix an issue, and it's testable: add a unit test to check that we don't regress and suffer from that bug again.
* If you create a type, write unit tests to test its constraints.

#### Error Handling

* Use `thiserror` to emit readable error messages from your functions.
* It's fine to use `?` and `anyhow` inside function chains; prefer `result.map_err` to transform your errors into your own errors whenever it's possible that an error can be returned from a function accessible beyond the immediate crate.
* Issue a `log::error!` or `log::warn!` messaage when an error occurs. Don't trust the callee to do it for you. It's better to have duplicate error messages than none at all.

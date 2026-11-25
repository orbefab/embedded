# orbe

A hardware simulator for the AI agent you already use.

orbe runs real firmware on simulated Arduino boards, so your agent can test a circuit before you build it. Your agent drives it from the command line, headless. You can watch the same circuit live in 3D. It works with any agent, on your own subscription. orbe is made by [Orbefab](https://orbefab.com), and its parts and Blocks are at [orbefab.com/orbe](https://orbefab.com/orbe/).

It runs the Arduino Nano and Uno today. ESP32 is in progress.

## Install

```sh
curl -fsSL https://orbefab.com/orbe/install.sh | sh
```

It installs `orbe` with the 3D viewer into `~/.local/bin`, on Linux x86_64 with glibc 2.35 or newer (Ubuntu 22.04, Debian 12 and later) and macOS arm64. `orbe add`, `orbe search` and `orbe part` need curl.

## Try it

```sh
mkdir demo && cd demo
orbe add nano-button-led
orbe check scene.jsonc
orbe test
orbe open --headed
orbe play
orbe close
```

`orbe search <words>` finds parts, and `orbe part <id>` prints one.

## Give it to your agent

```sh
orbe skills get core
```

That prints how an agent uses `orbe`, for the version installed. To install a small skill that tells your agents to run it:

```sh
npx skills add orbefab/orbe
```

## Contribute

[docs/contributing.md](docs/contributing.md) says how to build `orbe` from source, run the checks, and find your way around the repo.

## License

The code is under MIT OR Apache-2.0, at your option, unless a file says otherwise. The MIT license is [LICENSE-MIT](LICENSE-MIT). The Apache License, Version 2.0 is [LICENSE-APACHE](LICENSE-APACHE). A contribution you submit for inclusion is under the same two licenses, with no other terms.

Block firmware images link Arduino cores and libraries under their own licenses. Each Block's `firmware/NOTICE` lists them.

# RustyLine Async
[![Docs](https://docs.rs/rustyline-async/badge.svg)](https://docs.rs/rustyline-async)
[![](https://img.shields.io/crates/v/rustyline-async.svg)](https://crates.io/crates/rustyline-async)
![](https://tokei.rs/b1/github/zyansheep/rustyline-async?category=code)

A minimal readline with multiline and async support.

Inspired by [`rustyline`](https://crates.io/crates/rustyline),
[`async-readline`](https://crates.io/crates/async-readline), &
`termion-async-input`. Built using
[`crossterm`](https://crates.io/crates/crossterm).

## Features

 * Works on all platforms supported by `crossterm`.
 * Full Unicode Support (Including Grapheme Clusters)
 * Multiline Editing
 * In-memory History
 * Ctrl-C, Ctrl-D are returned as `Ok(Interrupt)` and `Ok(Eof)` `ReadlineEvent`s.
 * Ctrl-U to clear line before cursor
 * Ctrl-left & right to move to next or previous whitespace
 * Home/Ctrl-A and End/Ctrl-E to jump to the start and end of the input (Ctrl-A & Ctrl-E can be toggled off by disabling the "emacs" feature)
 * Ctrl-L clear screen
 * Ctrl-W delete until previous space
 * Extensible design based on `crossterm`'s `event-stream` feature

Feel free to PR to add more features!

## Example:
```
cargo run --example readline
```

![rustyline-async](https://i.imgur.com/Ei2bzgu.gif)

## License
This software is licensed under The Unlicense license.

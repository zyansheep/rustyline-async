# RustyLine Async
[![Docs](https://docs.rs/rustyline-async/badge.svg)](https://docs.rs/rustyline-async)
[![](https://img.shields.io/crates/v/rustyline-async.svg)](https://crates.io/crates/rustyline-async)
![](https://tokei.rs/b1/github/zyansheep/rustyline-async?category=code)

A minimal readline with multiline and async support.

Inspired by `rustyline` , `async-readline` & `termion-async-input`. Built using `crossterm`

## Features

 * Works on all platforms supported by `crossterm`.
 * Full Unicode Support (Including Graphene Clusters)
 * Multiline Editing
 * In-memory History
 * Ctrl-C, Ctrl-D are returned as `Err(Interrupt)` and `Err(Eof)` respectively.
 * Ctrl-U to clear line before cursor
 * Ctrl-left & right to move to next or previous whitespace
 * Home/Ctrl-A and End/Ctrl-E to jump to the start and end of the input (Ctrl-A & Ctrl-E can be toggled off with feature)
 * Ctrl-L clear screen
 * Extensible design based on `crossterm`'s `event-stream` feature

Feel free to PR to add more features!

## Example:
```
cargo run --package readline
```

![rustyline-async](https://i.imgur.com/Ei2bzgu.gif)

## License
This software is licensed under The Unlicense license.

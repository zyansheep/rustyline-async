# RustyLine Async
[![Docs](https://docs.rs/rustyline-async/badge.svg)](https://docs.rs/rustyline-async)
[![](https://img.shields.io/crates/v/rustyline-async.svg)](https://crates.io/crates/rustyline-async)
![](https://tokei.rs/b1/github/zyansheep/rustyline-async?category=code)


A minimal readline with multiline and async support.

Inspired by `rustyline-async` , `async-readline` & `termion-async-input`. Build using `crossterm`

## Features

 * Unicode
 * Multiline Editing
 * Ctrl-C, Ctrl-D
 * Ctrl-U to clear line before cursor
 * Ctrl-left & right to move between words
 * Ctrl-L clear screen

Feel free to PR to add more features!

## Example:
```
cargo run --package readline
```

![rustyline-async](https://i.imgur.com/Ei2bzgu.gif)

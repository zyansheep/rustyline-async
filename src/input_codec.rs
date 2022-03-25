use std::{io, iter::empty};

use bytes::{Buf, Bytes, BytesMut};
use futures::AsyncRead;
use futures_codec::{Decoder, FramedRead};
use termion::event::{self, Event, Key};

/// An iterator over input events and the bytes that define them
pub type EventStream<R> = FramedRead<R, EventsDecoder>;

pub fn event_stream<R: AsyncRead + Unpin>(reader: R) -> EventStream<R> {
	FramedRead::new(reader, EventsDecoder)
}

pub struct EventsDecoder;

impl Decoder for EventsDecoder {
	type Item = Event;
	type Error = io::Error;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		match src.len() {
			0 => Ok(None),
			1 => match src[0] {
				b'\x1B' => {
					src.advance(1);
					Ok(Some(Event::Key(Key::Esc)))
				}
				c => {
					if let Ok(res) = parse_event(c, &mut empty()) {
						src.advance(1);
						Ok(Some(res))
					} else {
						Ok(None)
					}
				}
			},
			_ => {
				let (off, res) = if let Some((c, cs)) = src.split_first() {
					let cur = Bytes::copy_from_slice(cs);
					let mut it = cur.into_iter().map(Ok);
					if let Ok(res) = parse_event(*c, &mut it) {
						(1 + cs.len() - it.len(), Ok(Some(res)))
					} else {
						(0, Ok(None))
					}
				} else {
					(0, Ok(None))
				};

				src.advance(off);
				res
			}
		}
	}
}

fn parse_event<I>(item: u8, iter: &mut I) -> Result<Event, io::Error>
where
	I: Iterator<Item = Result<u8, io::Error>>,
{
	let mut buf = vec![item];
	let result = {
		let mut iter = iter.inspect(|byte| {
			if let &Ok(byte) = byte {
				buf.push(byte);
			}
		});
		event::parse_event(item, &mut iter)
	};
	result
}
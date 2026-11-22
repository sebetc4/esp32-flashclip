//! Each command, sent as bytes, against a bus that records what it is asked.

mod common;

use common::{Bytes, Event, Recorder};
use flashclip_protocol::{ACK, Error, NAK, Programmer, command::*};

const N: usize = 16;

/// Serves every command of `input`, then checks that the link was read to its
/// end: each command consumed exactly its parameters.
fn run(input: &[u8]) -> (Vec<u8>, Vec<Event>) {
    let mut programmer = Programmer::<_, N>::new(Recorder::default());
    let mut link = Bytes::new(input);
    loop {
        match programmer.serve(&mut link) {
            Ok(()) => {}
            Err(Error::Closed) => break,
            Err(Error::Link(e)) => match e {},
        }
    }
    let events = std::mem::take(&mut programmer.bus().events);
    (link.output, events)
}

fn spiop(write: &[u8], rlen: u32) -> Vec<u8> {
    let slen = write.len() as u32;
    let mut bytes = vec![O_SPIOP];
    bytes.extend_from_slice(&slen.to_le_bytes()[..3]);
    bytes.extend_from_slice(&rlen.to_le_bytes()[..3]);
    bytes.extend_from_slice(write);
    bytes
}

#[test]
fn queries_answer_what_the_programmer_is() {
    let (output, events) = run(&[NOP, Q_IFACE, Q_PGMNAME, Q_SERBUF, Q_BUSTYPE]);
    let mut expected = vec![ACK, ACK, 1, 0, ACK];
    expected.extend_from_slice(b"flashclip\0\0\0\0\0\0\0");
    expected.extend_from_slice(&[ACK, 0xFF, 0xFF, ACK, 0x08]);
    assert_eq!(output, expected);
    assert!(events.is_empty());
}

#[test]
fn the_lengths_report_the_buffer_and_the_read_limit() {
    let (output, _) = run(&[Q_WRNMAXLEN, Q_RDNMAXLEN]);
    assert_eq!(output, [ACK, N as u8, 0, 0, ACK, 0, 0, 1]);
}

#[test]
fn the_command_map_holds_the_served_commands_only() {
    let (output, _) = run(&[Q_CMDMAP]);
    assert_eq!(output.len(), 33);
    assert_eq!(output[0], ACK);
    let map = &output[1..];
    let served = |opcode: u8| map[usize::from(opcode / 8)] & (1 << (opcode % 8)) != 0;
    for opcode in 0..=255u8 {
        assert_eq!(
            served(opcode),
            SUPPORTED.contains(&opcode),
            "opcode {opcode:#04x}"
        );
    }
    // What flashrom needs of an SPI programmer.
    for opcode in [Q_SERBUF, Q_WRNMAXLEN, O_SPIOP, S_BUSTYPE, S_SPI_FREQ] {
        assert!(served(opcode));
    }
}

#[test]
fn sync_nop_answers_nak_then_ack() {
    let (output, _) = run(&[SYNCNOP, SYNCNOP]);
    assert_eq!(output, [NAK, ACK, NAK, ACK]);
}

#[test]
fn unknown_and_parallel_commands_are_refused() {
    let (output, events) = run(&[O_INIT, 0x42, 0xFF]);
    assert_eq!(output, [NAK, NAK, NAK]);
    assert!(events.is_empty());
}

#[test]
fn the_bus_type_must_include_spi() {
    let (output, _) = run(&[S_BUSTYPE, 0x08, S_BUSTYPE, 0x0F, S_BUSTYPE, 0x01]);
    assert_eq!(output, [ACK, ACK, NAK]);
}

#[test]
fn the_frequency_set_is_the_one_the_bus_made() {
    let mut input = vec![S_SPI_FREQ];
    input.extend_from_slice(&8_000_000u32.to_le_bytes());
    input.push(S_SPI_FREQ);
    input.extend_from_slice(&80_000_000u32.to_le_bytes());
    input.push(S_SPI_FREQ);
    input.extend_from_slice(&0u32.to_le_bytes());
    let (output, events) = run(&input);

    let mut expected = vec![ACK];
    expected.extend_from_slice(&8_000_000u32.to_le_bytes());
    expected.push(ACK);
    expected.extend_from_slice(&40_000_000u32.to_le_bytes());
    expected.push(NAK);
    assert_eq!(output, expected);
    assert_eq!(
        events,
        [Event::Frequency(8_000_000), Event::Frequency(80_000_000)]
    );
}

#[test]
fn only_the_first_chip_select_exists() {
    let (output, _) = run(&[S_SPI_CS, 0, S_SPI_CS, 1]);
    assert_eq!(output, [ACK, NAK]);
}

#[test]
fn an_operation_writes_then_reads_in_one_selection() {
    let (output, events) = run(&spiop(&[0x9F], 3));
    assert_eq!(output, [ACK, 0, 1, 2]);
    assert_eq!(
        events,
        [
            Event::Drivers(true),
            Event::Select,
            Event::Write(vec![0x9F]),
            Event::Read(3),
            Event::Deselect,
        ]
    );
}

#[test]
fn a_write_without_a_read_is_acknowledged_after_it() {
    let (output, events) = run(&spiop(&[0x06], 0));
    assert_eq!(output, [ACK]);
    assert_eq!(
        &events[1..],
        [
            Event::Select,
            Event::Write(vec![0x06]),
            Event::Read(0),
            Event::Deselect
        ]
    );
}

#[test]
fn a_read_longer_than_the_buffer_is_streamed_in_one_selection() {
    let rlen = 3 * N + 5;
    let (output, events) = run(&spiop(&[0x03, 0, 0, 0], rlen as u32));
    assert_eq!(output.len(), 1 + rlen);
    assert_eq!(output[0], ACK);
    let expected: Vec<u8> = (0..rlen).map(|i| i as u8).collect();
    assert_eq!(output[1..], expected);
    assert_eq!(events.iter().filter(|e| **e == Event::Select).count(), 1);
    assert_eq!(events.last(), Some(&Event::Deselect));
    let read: usize = events
        .iter()
        .map(|e| if let Event::Read(n) = e { *n } else { 0 })
        .sum();
    assert_eq!(read, rlen);
}

#[test]
fn a_write_longer_than_the_buffer_is_refused_and_the_stream_kept() {
    let mut input = spiop(&[0xAA; N + 1], 4);
    input.push(NOP);
    let (output, events) = run(&input);
    assert_eq!(output, [NAK, ACK]);
    assert!(events.is_empty(), "nothing reaches the bus: {events:?}");
}

#[test]
fn a_write_as_long_as_the_buffer_is_taken() {
    let (output, events) = run(&spiop(&[0xAA; N], 0));
    assert_eq!(output, [ACK]);
    assert!(events.contains(&Event::Write(vec![0xAA; N])));
}

#[test]
fn the_pins_stay_released_until_the_host_drives_them() {
    let mut input = vec![S_PIN_STATE, 1, S_PIN_STATE, 0];
    input.extend(spiop(&[0x05], 1));
    let (output, events) = run(&input);
    assert_eq!(output, [ACK, ACK, ACK, 0]);
    assert_eq!(
        events[..3],
        [
            Event::Drivers(true),
            Event::Drivers(false),
            Event::Drivers(true)
        ]
    );
}

#[test]
fn the_chip_select_modes_hold_the_selection_across_operations() {
    let mut input = vec![S_PIN_STATE, 1, S_CS_MODE, 1];
    input.extend(spiop(&[0x03, 0, 0, 0], 2));
    input.extend(spiop(&[], 2));
    input.extend([S_CS_MODE, 0, S_CS_MODE, 3]);
    let (output, events) = run(&input);
    assert_eq!(output, [ACK, ACK, ACK, 0, 1, ACK, 2, 3, ACK, NAK]);
    assert_eq!(
        events,
        [
            Event::Drivers(true),
            Event::Select,
            Event::Write(vec![0x03, 0, 0, 0]),
            Event::Read(2),
            Event::Write(vec![]),
            Event::Read(2),
            Event::Deselect,
        ]
    );
}

#[test]
fn a_selection_held_comes_back_with_the_pins() {
    let (output, events) = run(&[S_CS_MODE, 1, S_PIN_STATE, 1, S_PIN_STATE, 0, S_PIN_STATE, 1]);
    assert_eq!(output, [ACK; 4]);
    assert_eq!(
        events,
        [
            Event::Drivers(true),
            Event::Select,
            Event::Drivers(false),
            Event::Drivers(true),
            Event::Select,
        ]
    );
}

#[test]
fn a_command_cut_short_closes_the_link() {
    let mut programmer = Programmer::<_, N>::new(Recorder::default());
    let mut link = Bytes::new(&[O_SPIOP, 1, 0]);
    assert_eq!(programmer.serve(&mut link), Err(Error::Closed));
    assert!(link.output.is_empty());
}

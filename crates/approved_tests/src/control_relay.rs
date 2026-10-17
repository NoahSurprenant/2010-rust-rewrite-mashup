//! Unit tests for the master's control relay bound, owner-approved
//! 2026-10-17: what a host owes a joining client (a long edit log) is cut
//! into relay messages that each fit `MAX_CONTROL_BYTES`, in order, and one
//! that cannot fit is refused before the stream writer sees it.

use master_protocol::{
    ControlFrame, MAX_CONTROL_BYTES, MAX_CONTROL_RELAY_PAYLOAD, MemberId, RelayDatagram,
    control_relay_runs, decode_relay_stream, encode_relay_control, encode_stream_frame,
};

/// What a Minecraft edit row takes on the wire at most: a 4 KiB batch of
/// changes plus its message and row headers.
const ROW: usize = 4 * 1024 + 16;
/// A packet's own bytes beside its rows.
const OVERHEAD: usize = 64;

fn relayed(payload_len: usize) -> Result<Vec<u8>, master_protocol::ProtocolError> {
    let payload = vec![0xa5; payload_len];
    let bytes = encode_relay_control(RelayDatagram::HostToMember {
        member_id: MemberId([7; 16]),
        payload: &payload,
    })?;
    encode_stream_frame(&ControlFrame::Relay(bytes))
}

#[test]
fn a_full_relay_message_fits_the_control_stream() {
    let frame = relayed(MAX_CONTROL_RELAY_PAYLOAD).unwrap();
    assert!(frame.len() - 4 <= MAX_CONTROL_BYTES);
}

#[test]
fn one_byte_more_is_refused_before_the_stream() {
    assert!(relayed(MAX_CONTROL_RELAY_PAYLOAD + 1).is_err());
}

#[test]
fn a_long_backlog_is_cut_into_messages_that_fit() {
    // The rows a client that joins a day-old world is owed, of mixed sizes.
    let sizes: Vec<usize> = (0..64).map(|i| [ROW, 9, 300, ROW / 2][i % 4]).collect();
    let runs = control_relay_runs(&sizes, OVERHEAD);
    assert_eq!(runs.iter().sum::<usize>(), sizes.len());
    assert!(runs.len() > 1);
    let mut at = 0;
    for run in runs {
        assert!(run > 0);
        let bytes: usize = sizes[at..at + run].iter().sum();
        assert!(OVERHEAD + bytes <= MAX_CONTROL_RELAY_PAYLOAD);
        let frame = relayed(OVERHEAD + bytes).unwrap();
        assert!(frame.len() - 4 <= MAX_CONTROL_BYTES);
        at += run;
    }
}

#[test]
fn rows_that_fit_one_message_stay_in_one() {
    assert_eq!(control_relay_runs(&[10, 20, 30], OVERHEAD), vec![3]);
    assert_eq!(control_relay_runs(&[], OVERHEAD), vec![0]);
    let budget = MAX_CONTROL_RELAY_PAYLOAD - OVERHEAD;
    let half = budget / 2;
    assert_eq!(
        control_relay_runs(&[half, budget - half], OVERHEAD),
        vec![2]
    );
    assert_eq!(
        control_relay_runs(&[half, budget - half, 1], OVERHEAD),
        vec![2, 1]
    );
}

#[test]
fn a_row_too_large_for_any_message_goes_alone() {
    let runs = control_relay_runs(&[5, MAX_CONTROL_RELAY_PAYLOAD, 5], OVERHEAD);
    assert_eq!(runs, vec![1, 1, 1]);
}

#[test]
fn the_master_reads_back_what_the_host_relays() {
    let payload = vec![3; 1234];
    let bytes = encode_relay_control(RelayDatagram::HostToMember {
        member_id: MemberId([9; 16]),
        payload: &payload,
    })
    .unwrap();
    match decode_relay_stream(&bytes).unwrap() {
        RelayDatagram::HostToMember { member_id, payload } => {
            assert_eq!(member_id, MemberId([9; 16]));
            assert_eq!(payload, &[3; 1234][..]);
        }
        _ => panic!("wrong relay direction"),
    }
}

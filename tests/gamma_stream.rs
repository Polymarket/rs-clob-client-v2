#![cfg(feature = "gamma")]

use std::cell::RefCell;

use futures::{StreamExt as _, executor::block_on, future};
use polymarket_client_sdk_v2::{error::Error, gamma::Client};

#[test]
fn stream_stops_on_empty_page_for_any_limit() {
    let client = Client::default();

    for limit in [-1, 0, 1] {
        let calls = RefCell::new(Vec::new());
        let stream = client.stream_data(
            |_, requested_limit, offset| {
                let mut calls = calls.borrow_mut();
                calls.push((requested_limit, offset));
                future::ready(if calls.len() == 1 {
                    Ok(Vec::<u8>::new())
                } else {
                    Err(Error::validation(
                        "requested another page after an empty page",
                    ))
                })
            },
            limit,
        );
        futures::pin_mut!(stream);

        assert!(block_on(stream.next()).is_none(), "limit: {limit}");
        assert_eq!(*calls.borrow(), vec![(limit, 0)]);
    }
}

#[test]
fn stream_stops_on_empty_page_after_returning_items() {
    let client = Client::default();

    for limit in [-1, 0, 1] {
        let calls = RefCell::new(Vec::new());
        let stream = client.stream_data(
            |_, requested_limit, offset| {
                let mut calls = calls.borrow_mut();
                calls.push((requested_limit, offset));
                future::ready(match calls.len() {
                    1 => Ok(vec![42]),
                    2 => Ok(Vec::new()),
                    _ => Err(Error::validation(
                        "requested another page after an empty page",
                    )),
                })
            },
            limit,
        );
        futures::pin_mut!(stream);

        let items: Vec<_> = block_on(stream.collect());
        assert_eq!(items.len(), 1, "limit: {limit}");
        assert!(matches!(items[0], Ok(42)));
        assert_eq!(*calls.borrow(), vec![(limit, 0), (limit, 1)]);
    }
}

#[test]
fn stream_stops_on_short_nonempty_page() {
    let client = Client::default();
    let calls = RefCell::new(Vec::new());
    let stream = client.stream_data(
        |_, limit, offset| {
            let mut calls = calls.borrow_mut();
            calls.push((limit, offset));
            future::ready(match calls.len() {
                1 => Ok(vec![1, 2]),
                2 => Ok(vec![3]),
                _ => Err(Error::validation(
                    "requested another page after a short page",
                )),
            })
        },
        2,
    );
    futures::pin_mut!(stream);

    let items: Vec<_> = block_on(stream.collect());
    assert_eq!(items.len(), 3);
    assert!(matches!(items[0], Ok(1)));
    assert!(matches!(items[1], Ok(2)));
    assert!(matches!(items[2], Ok(3)));
    assert_eq!(*calls.borrow(), vec![(2, 0), (2, 2)]);
}

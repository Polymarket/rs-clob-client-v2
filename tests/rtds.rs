#![cfg(feature = "rtds")]

use polymarket_client_sdk_v2::rtds::subscription::SimpleParser;

#[test]
fn simple_parser_can_be_constructed_downstream() {
    let _: SimpleParser = SimpleParser::default();
}

use super::*;
use futures::SinkExt;
use nodelite_proto::{HelloMessage, NodeIdentity, WIRE_PROTOCOL_VERSION, WireMessage};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::{Frame, coding::Data, coding::OpCode};

const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const FRAGMENT_BYTES: usize = 4 * 1024;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticates_agent_and_exposes_node_over_http() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-handshake-01", "Integration Handshake 01")
        .await?;

    let mut agent = TestAgent::connect(&server, &node).await?;
    agent.send_fake_metrics(1).await?;

    let status = server
        .wait_for_node_uptime(&node.node_id, 1, TEST_TIMEOUT)
        .await?;
    assert!(status.online);
    assert_eq!(status.identity.node_label, node.node_label);

    let overview = server.overview().await?;
    assert_eq!(overview.total_nodes, 1);
    assert_eq!(overview.online_nodes, 1);

    let node_status = server.node_status(&node.node_id).await?;
    assert_eq!(node_status.identity.node_id, node.node_id);
    assert!(node_status.online);

    let nodes = server.nodes().await?;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].identity.node_id, node.node_id);

    agent.disconnect().await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepts_message_at_exact_64_kib_boundary() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-boundary-01", "Boundary 01")
        .await?;
    let payload = padded_hello_payload(&node);
    let (mut socket, _) = connect_async(format!("ws://{}/ws", server.addr)).await?;

    socket.send(Message::Text(payload.into())).await?;
    crate::test_support::wait_for_authenticated_notice(&mut socket, &node.node_id, TEST_TIMEOUT)
        .await?;

    socket.close(None).await?;
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepts_fragmented_message_at_exact_64_kib_boundary() -> Result<()> {
    let server = TestServer::start().await?;
    let node = server
        .issue_node("itest-fragmented-01", "Fragmented 01")
        .await?;
    let payload = padded_hello_payload(&node).into_bytes();
    let fragment_count = payload.len().div_ceil(FRAGMENT_BYTES);
    let (mut socket, _) = connect_async(format!("ws://{}/ws", server.addr)).await?;

    for (index, fragment) in payload.chunks(FRAGMENT_BYTES).enumerate() {
        let opcode = if index == 0 {
            OpCode::Data(Data::Text)
        } else {
            OpCode::Data(Data::Continue)
        };
        let frame = Frame::message(fragment.to_vec(), opcode, index + 1 == fragment_count);
        socket.send(Message::Frame(frame)).await?;
    }
    crate::test_support::wait_for_authenticated_notice(&mut socket, &node.node_id, TEST_TIMEOUT)
        .await?;

    socket.close(None).await?;
    server.shutdown().await
}

fn padded_hello_payload(node: &crate::test_support::TestNode) -> String {
    let hello = WireMessage::Hello(HelloMessage {
        protocol_version: WIRE_PROTOCOL_VERSION,
        token: node.token.clone(),
        identity: NodeIdentity {
            node_id: node.node_id.clone(),
            node_label: node.node_label.clone(),
            hostname: format!("{}.test", node.node_id),
            os: "test".to_string(),
            kernel_version: None,
            cpu_model: None,
            cpu_cores: 1,
            agent_version: "integration-test".to_string(),
            boot_time: None,
            tags: Vec::new(),
        },
    });
    let mut payload = serde_json::to_string(&hello).expect("hello should serialize");
    assert!(payload.len() <= MAX_MESSAGE_BYTES);
    payload.push_str(&" ".repeat(MAX_MESSAGE_BYTES - payload.len()));
    payload
}

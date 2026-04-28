use raftust_core::{RaftNode, RequestVoteResponse, Role};

fn main() {
    let mut node = RaftNode::new(1, vec![2, 3], 5, 2);
    let vote_requests = node.start_election();

    println!("Started election for term {}", node.current_term);
    println!("Broadcast {} RequestVote RPCs", vote_requests.len());

    let became_leader = node.handle_request_vote_response(RequestVoteResponse {
        term: node.current_term,
        vote_granted: true,
        from: 2,
    });

    if became_leader && node.role == Role::Leader {
        println!("Node {} became leader", node.id);
    }
}

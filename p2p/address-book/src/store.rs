use std::fs;
use std::collections::HashMap;

use borsh::{from_slice, to_vec, BorshDeserialize, BorshSerialize};
use tokio::task::{spawn_blocking, JoinHandle};

use tokio::time::Instant;

use cuprate_p2p_core::{services::ZoneSpecificPeerListEntryBase, NetZoneAddress};

use crate::{peer_list::PeerList, AddressBookConfig, BorshNetworkZone};

// TODO: store anchor and ban list.

#[derive(BorshSerialize)]
struct SerPeerDataV1<'a, A: NetZoneAddress> {
    white_list: Vec<&'a ZoneSpecificPeerListEntryBase<A>>,
    gray_list: Vec<&'a ZoneSpecificPeerListEntryBase<A>>,
    banned_peers: HashMap<<A as NetZoneAddress>::BanID, u64>,
}


#[derive(BorshDeserialize)]
struct DeserPeerDataV1<A: NetZoneAddress> {
    white_list: Vec<ZoneSpecificPeerListEntryBase<A>>,
    gray_list: Vec<ZoneSpecificPeerListEntryBase<A>>,
    banned_peers: HashMap<<A as NetZoneAddress>::BanID, u64>,
}



pub fn save_peers_to_disk<Z: BorshNetworkZone>(
    cfg: &AddressBookConfig,
    white_list: &PeerList<Z>,
    gray_list: &PeerList<Z>,
    banned_peers: &HashMap<<Z::Addr as NetZoneAddress>::BanID, Instant>,
) -> JoinHandle<std::io::Result<()>> {
    // maybe move this to another thread but that would require cloning the data ... this
    // happens so infrequently that it's probably not worth it.
    let data = SerPeerDataV1 {
        white_list: white_list.peers.values().collect(),
        gray_list: gray_list.peers.values().collect(),
        banned_peers: banned_peers.iter().map(|(&ban_id, &instant)| {
            (ban_id, instant.duration_since(Instant::now()).as_millis() as u64)
        }).collect(),
    };

    let file = cfg.peer_store_file.clone();
    spawn_blocking(move || fs::write(&file, &data))
}

pub async fn read_peers_from_disk<Z: BorshNetworkZone>(
    cfg: &AddressBookConfig,
) -> Result<
    (
        Vec<ZoneSpecificPeerListEntryBase<Z::Addr>>,
        Vec<ZoneSpecificPeerListEntryBase<Z::Addr>>,
    ),
    std::io::Error,
> {
    let file = cfg.peer_store_file.clone();
    let data = spawn_blocking(move || fs::read(file)).await??;

    let de_ser: DeserPeerDataV1<Z::Addr> = from_slice(&data)?;

    Ok((de_ser.white_list, de_ser.gray_list))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_list::{tests::{make_fake_peer_list,make_fake_peer_list_with_bans}, PeerList};
    use cuprate_test_utils::test_netzone::{TestNetZone, TestNetZoneAddr};
    use std::time::{Instant, Duration};

    #[test]
    fn ser_deser_peer_list() {
        let (white_list, mut banned_peers) = make_fake_peer_list_with_bans(50);
        let gray_list = make_fake_peer_list(50, 50);

        // Add some more bans for testing
        for _ in 0..10 {
            let peer = make_fake_peer(banned_peers.peers.len() as u32, None);
            banned_peers.peers.insert(peer.adr.clone(), peer);
            banned_peers.ban_ids.insert(peer.adr.ban_id(), vec![peer.adr.clone()]);
            banned_peers.insert(peer.adr.ban_id(), Instant::now() + Duration::from_secs(3600));
        }

        let data = to_vec(&SerPeerDataV1 {
            white_list: white_list.peers.values().collect::<Vec<_>>(),
            gray_list: gray_list.peers.values().collect::<Vec<_>>(),
            banned_peers: banned_peers.iter().map(|(ban_id, &instant)| {
                (*ban_id, instant.duration_since(Instant::now()).as_secs() as u64)
            }).collect(),
        }).unwrap();

        let de_ser: DeserPeerDataV1<TestNetZoneAddr> = from_slice(&data).unwrap();

        let white_list_2: PeerList<TestNetZone<true, true, true>> = PeerList::new(de_ser.white_list);
        let gray_list_2: PeerList<TestNetZone<true, true, true>> = PeerList::new(de_ser.gray_list);
        let mut banned_peers_2 = HashMap::new();

        for (ban_id, timestamp) in de_ser.banned_peers {
            banned_peers_2.insert(ban_id, Instant::now() + Duration::from_secs(timestamp));
        }

        // Test white and gray lists
        assert_eq!(white_list.peers.len(), white_list_2.peers.len());
        assert_eq!(gray_list.peers.len(), gray_list_2.peers.len());

        for addr in white_list.peers.keys() {
            assert!(white_list_2.contains_peer(addr));
        }

        for addr in gray_list.peers.keys() {
            assert!(gray_list_2.contains_peer(addr));
        }

        // Test banned peers
        assert_eq!(banned_peers.len(), banned_peers_2.len());

        for (ban_id, ban_until) in banned_peers.iter() {
            let ban_until_2 = banned_peers_2.get(ban_id).unwrap();
            assert!(ban_until.duration_since(*ban_until_2).abs() < Duration::from_secs(1));
        }
    }
}

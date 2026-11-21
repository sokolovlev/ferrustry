//! Сессия через настоящий UDP (renet, D-08) на 127.0.0.1: сервер и два клиента в одном процессе.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use ferrustry_content::ContentRegistry;
use ferrustry_net::{
    Client, ClientEvent, Server, ServerConfig, ServerEvent, UdpHub, UdpLink, UdpPeer,
};
use ferrustry_sim::rng::Rng;
use ferrustry_sim::{Command, Dir, TilePos, World, scenario, state_hash};

#[test]
fn two_clients_over_udp_stay_identical() {
    let reg = ContentRegistry::base().unwrap();
    // мегабаза 20k: снимок в сотни КБ — проверяет нарезку больших сообщений
    let world = scenario::bench_megabase(&reg, 20_000).unwrap();
    let hub = UdpHub::bind("127.0.0.1:0".parse().unwrap(), 8).unwrap();
    let addr = hub.local_addr().unwrap();
    let config = ServerConfig {
        checksum_every: 30,
        ..Default::default()
    };
    let mut server: Server<UdpPeer> = Server::new(world, config);
    let mut clients: Vec<(Client<UdpLink>, Option<World>)> = (0..2)
        .map(|i| {
            let link = UdpLink::connect(addr).unwrap();
            (
                Client::connect(link, &reg, &format!("udp{i}")).unwrap(),
                None,
            )
        })
        .collect();
    let conveyor = reg.block_by_name("conveyor").unwrap();
    let mut rng = Rng::new(3);
    let start = Instant::now();
    let mut last = Instant::now();
    let mut ticks_after_join = 0;
    let mut joined_events = 0;
    while ticks_after_join < 400 {
        assert!(start.elapsed() < Duration::from_secs(60), "timed out");
        let dt = last.elapsed();
        last = Instant::now();
        hub.update(dt).unwrap();
        for p in hub.accept() {
            server.accept(p);
        }
        for e in server.poll(&reg) {
            assert!(matches!(e, ServerEvent::Joined { .. }), "{e:?}");
            joined_events += 1;
        }
        server.tick(&reg);
        if joined_events == 2 {
            ticks_after_join += 1;
        }
        hub.flush();
        for (c, w) in &mut clients {
            c.link_mut().update(dt).unwrap();
            for e in c.poll(&reg) {
                match e {
                    ClientEvent::Joined(_, snapshot) => *w = Some(*snapshot),
                    other => panic!("{other:?}"),
                }
            }
            if let Some(w) = w {
                let (_, ev) = c.advance(&reg, w, usize::MAX);
                assert!(
                    ev.iter().all(|e| matches!(e, ClientEvent::Results(_))),
                    "{ev:?}"
                );
                if ticks_after_join < 350 && rng.below(4) == 0 {
                    let p = TilePos::new(rng.below(120) as i32, rng.below(120) as i32);
                    c.send(Command::Place {
                        block: conveyor,
                        origin: p,
                        rot: Dir::Up,
                    })
                    .unwrap();
                }
            }
            c.link_mut().flush().unwrap();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // догнать: сервер стоит, клиенты дочитывают кадры
    let target = server.world().tick_count();
    while clients
        .iter()
        .any(|(_, w)| w.as_ref().is_none_or(|w| w.tick_count() < target))
    {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "catch-up timed out"
        );
        let dt = last.elapsed();
        last = Instant::now();
        hub.update(dt).unwrap();
        hub.flush();
        for (c, w) in &mut clients {
            c.link_mut().update(dt).unwrap();
            assert!(c.poll(&reg).is_empty());
            let (_, ev) = c.advance(&reg, w.as_mut().unwrap(), usize::MAX);
            assert!(
                ev.iter().all(|e| matches!(e, ClientEvent::Results(_))),
                "{ev:?}"
            );
            c.link_mut().flush().unwrap();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let h = state_hash(server.world());
    for (i, (c, w)) in clients.iter().enumerate() {
        assert_eq!(state_hash(w.as_ref().unwrap()), h, "client {i} diverged");
        assert!(c.verified() >= 10, "client {i} verified {}", c.verified());
        assert_eq!(c.resyncs(), 0);
    }
    assert_eq!(hub.connected(), 2);
}

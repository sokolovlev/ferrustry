//! Сессия «сервер + клиенты» в одном процессе (S6-02): у всех одинаковое состояние.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrustry_content::{BlockId, ContentRegistry};
use ferrustry_net::{
    Client, ClientEvent, Link, MemLink, Server, ServerConfig, ServerEvent, mem_pair,
};
use ferrustry_sim::mapgen::{self, Difficulty, MapSettings, MapSize};
use ferrustry_sim::rng::Rng;
use ferrustry_sim::{Command, Dir, Rules, TilePos, World, state_hash};

fn world(reg: &ContentRegistry) -> World {
    let mut w = mapgen::generate(
        reg,
        &MapSettings {
            size: MapSize::Small,
            difficulty: Difficulty::Hard,
            seed: 5,
        },
    )
    .unwrap();
    // бесплатная стройка и ранние волны: команды чаще проходят, идёт бой
    w.set_rules(Rules {
        infinite_resources: true,
        waves: true,
        first_wave: 300,
        wave_spacing: 400,
    });
    // враги с самого начала: в партии идёт бой
    let crawler = reg.unit_by_name("crawler").unwrap();
    let spawns = w.spawns().to_vec();
    for (i, sp) in spawns.iter().enumerate() {
        for k in 0..10 {
            w.spawn_unit(
                reg,
                crawler,
                sp.x * 256 + 64 * k,
                sp.y * 256 + 32 * i as i32,
            );
        }
    }
    w
}

fn random_command(rng: &mut Rng, blocks: &[BlockId]) -> Command {
    let p = TilePos::new(
        64 - 16 + rng.below(32) as i32,
        48 - 16 + rng.below(32) as i32,
    );
    if rng.below(4) == 0 {
        Command::Remove { at: p }
    } else {
        Command::Place {
            block: blocks[rng.below(blocks.len() as u32) as usize],
            origin: p,
            rot: [Dir::Right, Dir::Up, Dir::Left, Dir::Down][rng.below(4) as usize],
        }
    }
}

#[test]
fn server_and_three_clients_stay_identical() {
    let reg = ContentRegistry::base().unwrap();
    let config = ServerConfig {
        checksum_every: 60,
        ..Default::default()
    };
    let mut server: Server<MemLink> = Server::new(world(&reg), config);
    let blocks: Vec<BlockId> = [
        "conveyor",
        "bore",
        "smelter",
        "wind_turbine",
        "power_node",
        "gun_turret",
        "arc_tower",
        "iron_wall",
        "splitter",
        "battery",
    ]
    .iter()
    .map(|n| reg.block_by_name(n).unwrap())
    .collect();
    let mut clients: Vec<Client<MemLink>> = Vec::new();
    let joins = [0u64, 150, 700]; // третий входит в идущую партию
    let mut rng = Rng::new(99);
    let mut checks_seen = 0;
    for step in 0..2000u64 {
        if let Some(i) = joins.iter().position(|&t| t == step) {
            let (s, c) = mem_pair();
            server.accept(s);
            clients.push(Client::connect(c, &reg, &format!("p{i}")).unwrap());
        }
        for c in &mut clients {
            if c.world().is_some() && rng.below(3) == 0 {
                c.send(random_command(&mut rng, &blocks)).unwrap();
            }
        }
        for e in server.poll(&reg) {
            assert!(
                matches!(e, ServerEvent::Joined { .. }),
                "unexpected server event {e:?}"
            );
        }
        server.tick(&reg);
        for (i, c) in clients.iter_mut().enumerate() {
            for e in c.poll(&reg) {
                assert!(matches!(e, ClientEvent::Joined(_)), "client {i}: {e:?}");
            }
            // последний клиент считает медленнее и догоняет пачками
            let max = if i == 2 { 3 } else { usize::MAX };
            let (_, events) = c.advance(&reg, max);
            assert!(events.is_empty(), "client {i}: {events:?}");
        }
        if step % 60 == 0 {
            checks_seen += 1;
        }
    }
    // дать отставшему дорассчитать
    for c in &mut clients {
        let (_, events) = c.advance(&reg, usize::MAX);
        assert!(events.is_empty());
    }
    let h = state_hash(server.world());
    assert_eq!(server.players(), 3);
    for (i, c) in clients.iter().enumerate() {
        let w = c.world().unwrap();
        assert_eq!(w.tick_count(), server.world().tick_count(), "client {i}");
        assert_eq!(state_hash(w), h, "client {i} diverged");
        assert!(!c.desynced());
    }
    // партия была содержательной: стройка прошла, бой был
    assert!(server.world().buildings().len() > 100);
    assert!(server.world().kills() > 0, "combat happened");
    assert!(checks_seen > 30);
}

#[test]
fn incompatible_clients_are_rejected() {
    let reg = ContentRegistry::base().unwrap();
    let mut server: Server<MemLink> = Server::new(world(&reg), ServerConfig::default());
    // другие числа в контенте — другой отпечаток
    let other = ContentRegistry::from_ron(&ferrustry_content::BASE_CONTENT.replacen(
        "damage: 12, reload: 15",
        "damage: 13, reload: 15",
        1,
    ))
    .unwrap();
    let (s, c) = mem_pair();
    server.accept(s);
    let mut client = Client::connect(c, &other, "modded").unwrap();
    let ev = server.poll(&reg);
    assert!(matches!(&ev[..], [ServerEvent::Rejected(why)] if why.contains("content")));
    let ev = client.poll(&other);
    assert!(matches!(&ev[..], [ClientEvent::Rejected(_), ..]));
    assert_eq!(server.players(), 0);

    // сервер полон
    let mut server: Server<MemLink> = Server::new(
        world(&reg),
        ServerConfig {
            max_players: 1,
            ..Default::default()
        },
    );
    let mut keep = Vec::new();
    for name in ["a", "b"] {
        let (s, c) = mem_pair();
        server.accept(s);
        keep.push(Client::connect(c, &reg, name).unwrap());
    }
    let ev = server.poll(&reg);
    assert!(matches!(&ev[0], ServerEvent::Joined { .. }));
    assert!(matches!(&ev[1], ServerEvent::Rejected(why) if why.contains("full")));
}

#[test]
fn desync_is_detected_at_the_next_checksum() {
    let reg = ContentRegistry::base().unwrap();
    let config = ServerConfig {
        checksum_every: 30,
        ..Default::default()
    };
    let mut server: Server<MemLink> = Server::new(world(&reg), config);
    let (s, c) = mem_pair();
    server.accept(s);
    let mut client = Client::connect(c, &reg, "p").unwrap();
    server.poll(&reg);
    client.poll(&reg);
    // «ошибка детерминизма»: клиент тайком получил предмет
    let ingot = reg.item_by_name("iron_ingot").unwrap();
    client.world_mut().unwrap().grant(&reg, ingot, 1);
    let mut desync = None;
    for _ in 0..40 {
        server.tick(&reg);
        client.poll(&reg);
        let (_, ev) = client.advance(&reg, usize::MAX);
        if let Some(ClientEvent::Desync { tick, .. }) = ev.first() {
            desync = Some(*tick);
        }
    }
    assert_eq!(desync, Some(30), "reported at the first checksum");
    assert!(client.desynced());
}

#[test]
fn misbehaving_clients_are_dropped() {
    let reg = ContentRegistry::base().unwrap();
    let mut server: Server<MemLink> = Server::new(world(&reg), ServerConfig::default());
    // мусор вместо Hello
    let (s, mut c) = mem_pair();
    server.accept(s);
    c.send(vec![0xFF, 0x00, 0x13]).unwrap();
    assert!(matches!(
        &server.poll(&reg)[..],
        [ServerEvent::Misbehaved(_)]
    ));
    // команда с несуществующей постройкой
    let (s, c) = mem_pair();
    server.accept(s);
    let mut client = Client::connect(c, &reg, "x").unwrap();
    server.poll(&reg);
    client
        .send(Command::Place {
            block: BlockId(9999),
            origin: TilePos::new(1, 1),
            rot: Dir::Right,
        })
        .unwrap();
    let ev = server.poll(&reg);
    assert!(matches!(&ev[0], ServerEvent::Misbehaved(why) if why.contains("9999")));
    assert!(matches!(&ev[1], ServerEvent::Left(_)));
    assert_eq!(server.players(), 0);
}

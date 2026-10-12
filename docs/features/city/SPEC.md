# city — towns, roads, road growth, traffic, mode choice
Crate `crates/city` (pure Rust, no ECS). Design: docs/arch/city-gen.md. Owner: mgr-city. Glue: crates/app/src/{layout,zones,roads}.rs.

## API
- `plan_towns(seed, regions) -> TownPlan { graph: RoadGraph, parcels: Vec<Parcel>, towns }`. Parcel index == growth ZoneId. `Parcel.access` = frontage edge.
- `RoadBuilder` (roadbuild.rs): city road growth, D-7. Rate = GROWTH_RATE * ROADS_PER_RATE; plans ghost LEAD_TICKS before building; `growth::RoadAgency` pulls roads toward underserved built zones.
- `Traffic` (traffic.rs): `assign(built, zones, transit_times, hour)` -> edge flows (BPR, smoothed), per-zone transit share, car/transit trips, VKT, mean trip time, accessibility, equity.
- `transit_times(net, zone_centres, stations)` (transit_times.rs): door-to-door transit minutes + line capacity.
- App: `GrowthSimulation` (sim thread) owns zones + builder + traffic; snapshot carries built edges, pending ghosts, flow/vc/slowdown per edge, zone shares, scorecard.

## Rules
- A zone grows only if its frontage road is built. Starting populated zones start with their road.
- Roads are the transit Road graph: built edges and zone access links are mirrored as `TransitAction::AddEdge`; transit node ids: zone i = i, road node n = zones + n, player stations after.
- Tunables (one place each): `zones::GROWTH_RATE` (pace of everything), `city::ROADS_PER_RATE`, traffic.rs constants (speeds, capacities, BPR, mode logit, trip rates, DAY_TICKS).

## Acceptance (tests)
- roadgen: same seed same graph; no road on water except arterial bridges (<= 22 u); towns connected; junction/cul-de-sac mix; parcels on land, clear of roads, with access; land use follows access; seed kinds assigned.
- roadbuild: builds outward connected at the set rate; stalls behind undeveloped streets, failsafe resumes; underserved zones pull roads; deterministic.
- traffic: congestion raises car travel time (and rush > night); better transit lowers car share; a line serving a busy origin takes cars off the road; unbuilt roads carry nothing; deterministic.
- app zones: every developed zone has road access; 5-minute growth bound; transit speeds growth; ghosts then roads then traffic appear; scorecard normalised.
- app roads: tint runs green to red; fewer cars when flow drops, capped; cars queue downstream when congested.

## Not verifiable without a screen
Look of generated towns in the real renderer, tint legibility, car dot density/size, queue appearance, road stroke widths vs rail.

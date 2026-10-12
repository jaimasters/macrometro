# economy DECISIONS
(format: D-n | date | decision | why | rejected)
- D-1 | 2026-10-12 | AgencyId imported from sim_core, not redefined | coordinator request, single shared type | local type
- D-2 | 2026-10-12 | road_reserve is an input (growth demand) clamped to max_road_reserve share | growth RoadAgency owns the plans | economy-owned underserved score
- D-3 | 2026-10-12 | Favouritism = exponent 1+car_share on performance plus RoadAdjacent earmark | D-6/D-8: weight carries favouritism | separate bias table
- D-4 | 2026-10-12 | Params (base_revenue, penalties, land rates) provisional | no spec numbers | none

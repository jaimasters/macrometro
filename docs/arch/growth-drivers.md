# Growth drivers
- Per zone accessibility A_i = sum_j jobs_j * exp(-beta*C[i][j]) (C from aggregation.md).
- Growth rate g_i = f(A_i, land availability, crowding penalty, transit share).
- Crowding: if line load factor > 1.0 on lines serving i, demand diverts to road mode -> road congestion -> C rises.
- Transit share s_i = transit trips / all trips from i. Low s_i over time triggers city auto-roads (proposals/city-agency.md) -> sprawl -> LA spiral.
- Generative sprawl: new development placed in chunks by A_i gradient (world_gen consumes growth demand).
- Independent per zone given C -> parallel (parallelism.md).

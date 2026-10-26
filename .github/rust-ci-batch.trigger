2026-10-24 single consolidated post-GPU/model validation: compiler E0599 fix and targeted retry.
The first run 40101291764 passed Linux world/CLI but desktop failed on i64.next_power_of_two.
Revalidate Linux and Windows cargo check/test only. No app builds or packages.

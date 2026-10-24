2026-10-24 corrected five-feature read-only scene 32x32x48 boundary verification.
Confirm height 48 allows Y -64..-17; height 49 Y -64..-16 rejects; width 33 rejects. cargo check/test only, no packaging.

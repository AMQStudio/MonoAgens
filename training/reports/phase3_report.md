# Phase 3 Full Multi-Task Report (Phase 1 + Phase 2 + `p_encode_importance` + `p_memory_relevance`)

- **Device:** `NVIDIA GeForce RTX 4060 Laptop GPU`
- **Split Sequences (P1+P2+P3):** train=6810, cal=880, test=1063
- **Winner:** `1m` + `gated_deltanet2` (`blake3=d169d7ee8731d577713704a1b96aa893ce907a06da8c2f22dfdd27e55ca537e4`)

| Scale | Cell | Params | T | P1 F1 | Gate F1 | ECE | TC-ECE | Unc MAE | Verify F1 | EncImp F1 | MemRel F1 | Rel Top-1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150k | mingru | 169,721 | 1.110 | 0.4003 | 0.7955 | 0.0420 | 0.0428 | 0.0080 | 0.8380 | 0.9935 | 0.9979 | 1.0000 |
| 150k | gated_deltanet2 | 175,929 | 1.107 | 0.4173 | 0.7983 | 0.0428 | 0.0421 | 0.0081 | 0.8369 | 0.9952 | 0.9959 | 1.0000 |
| 3m | mingru | 3,171,945 | 1.190 | 0.4323 | 0.7954 | 0.0382 | 0.0384 | 0.0059 | 0.8381 | 0.9970 | 0.9959 | 1.0000 |
| 1m | gated_deltanet2 | 1,219,689 | 1.181 | 0.4467 | 0.7990 | 0.0399 | 0.0395 | 0.0055 | 0.8455 | 0.9976 | 0.9959 | 1.0000 |

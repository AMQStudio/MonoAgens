# Phase 3 Full Multi-Task Report (Phase 1 + Phase 2 + `p_encode_importance` + `p_memory_relevance`)

- **Device:** `NVIDIA GeForce RTX 4060 Laptop GPU`
- **Split Sequences (P1+P2+P3):** train=6170, cal=800, test=983
- **Winner:** `1m` + `gated_deltanet2` (`blake3=db249f159d01490a4b75c7c45c690ef75485b514f46613b874f55f91c234e71d`)

| Scale | Cell | Params | T | P1 F1 | Gate F1 | ECE | TC-ECE | Unc MAE | Verify F1 | EncImp F1 | MemRel F1 | Rel Top-1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150k | mingru | 169,721 | 1.222 | 0.4387 | 0.7906 | 0.0282 | 0.0284 | 0.0084 | 0.8515 | 0.9917 | 0.9969 | 1.0000 |
| 150k | gated_deltanet2 | 175,929 | 1.131 | 0.4583 | 0.7927 | 0.0310 | 0.0324 | 0.0078 | 0.8463 | 0.9941 | 0.9969 | 1.0000 |
| 3m | mingru | 3,171,945 | 1.254 | 0.4630 | 0.7949 | 0.0291 | 0.0298 | 0.0064 | 0.8169 | 0.9970 | 0.9979 | 1.0000 |
| 1m | gated_deltanet2 | 1,219,689 | 1.177 | 0.4804 | 0.7949 | 0.0290 | 0.0296 | 0.0061 | 0.8782 | 0.9959 | 0.9969 | 1.0000 |

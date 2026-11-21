# Phase 3 Full Multi-Task Report (Phase 1 + Phase 2 + `p_encode_importance` + `p_memory_relevance`)

- **Device:** `NVIDIA GeForce RTX 4060 Laptop GPU`
- **Split Sequences (P1+P2+P3):** train=6810, cal=880, test=1063
- **Winner:** `150k` + `mingru` (`blake3=82898fa437b4b229feeeefe4f718fc96872a0af49bbfb466f9b8827c17db792d`)

| Scale | Cell | Params | T | P1 F1 | Gate F1 | ECE | TC-ECE | Unc MAE | Verify F1 | EncImp F1 | MemRel F1 | Rel Top-1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150k | mingru | 169,721 | 2.686 | 0.4690 | 0.8037 | 0.0239 | 0.0248 | 0.0088 | 0.8795 | 0.9882 | 0.9969 | 1.0000 |
| 150k | gated_deltanet2 | 175,929 | 2.591 | 0.4567 | 0.8004 | 0.0247 | 0.0261 | 0.0085 | 0.8698 | 0.9750 | 0.9959 | 1.0000 |
| 3m | mingru | 3,171,945 | 2.919 | 0.4845 | 0.7997 | 0.0221 | 0.0243 | 0.0062 | 0.8271 | 0.9953 | 0.9979 | 1.0000 |
| 1m | gated_deltanet2 | 1,219,689 | 2.715 | 0.4639 | 0.7974 | 0.0251 | 0.0263 | 0.0058 | 0.8192 | 0.9935 | 0.9959 | 1.0000 |

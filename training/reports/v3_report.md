# Phase 3 Full Multi-Task Report (schema=3, v22_heads=True, h_actor_dim=24)

- **Device:** `NVIDIA GeForce RTX 4060 Laptop GPU`
- **Split Sequences:** train=7674, cal=1024, test=1175
- **Winner:** `150k` + `gated_deltanet2` (`blake3=2e9e3794c79885e50a0f1081fdb454e80408f409da7ef7178ea1bdd61d4f0d4d`)

| Scale | Cell | Params | T | P1 F1 | Gate F1 | ECE | TC-ECE | Unc MAE | Verify F1 | EncImp F1 | MemRel F1 | Rel Top-1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150k | mingru | 186,720 | 2.283 | 0.7247 | 0.8729 | 0.0156 | 0.0145 | 0.0110 | 0.8195 | 0.9770 | 0.9980 | 1.0000 |
| 150k | gated_deltanet2 | 195,256 | 2.343 | 0.7275 | 0.8741 | 0.0133 | 0.0136 | 0.0104 | 0.8400 | 0.9823 | 0.9980 | 1.0000 |
| 3m | mingru | 3,234,432 | 2.907 | 0.7174 | 0.8734 | 0.0153 | 0.0132 | 0.0074 | 0.8389 | 0.9932 | 0.9980 | 1.0000 |
| 1m | gated_deltanet2 | 1,279,992 | 2.660 | 0.6920 | 0.8671 | 0.0160 | 0.0145 | 0.0075 | 0.8496 | 0.9949 | 0.9970 | 1.0000 |

# Phase 3 Full Multi-Task Report (schema=2, v22_heads=True, h_actor_dim=24)

- **Device:** `NVIDIA GeForce RTX 4060 Laptop GPU`
- **Split Sequences:** train=7674, cal=1024, test=1175
- **Winner:** `3m` + `mingru` (`blake3=31b8e77b53bf8b96cc70e48789c8c6ad050b651f4a11623c6e7b636062bdea89`)

| Scale | Cell | Params | T | P1 F1 | Gate F1 | ECE | TC-ECE | Unc MAE | Verify F1 | EncImp F1 | MemRel F1 | Rel Top-1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150k | mingru | 184,349 | 2.641 | 0.4306 | 0.8129 | 0.0230 | 0.0239 | 0.0107 | 0.8350 | 0.9831 | 0.9970 | 1.0000 |
| 150k | gated_deltanet2 | 192,885 | 2.678 | 0.4237 | 0.8066 | 0.0278 | 0.0286 | 0.0103 | 0.8498 | 0.9728 | 0.9950 | 1.0000 |
| 3m | mingru | 3,225,213 | 2.795 | 0.4457 | 0.8096 | 0.0262 | 0.0273 | 0.0073 | 0.8705 | 0.9926 | 0.9970 | 1.0000 |
| 1m | gated_deltanet2 | 1,271,061 | 2.756 | 0.4517 | 0.8145 | 0.0223 | 0.0237 | 0.0075 | 0.8369 | 0.9937 | 0.9980 | 1.0000 |

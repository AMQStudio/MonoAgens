# Phase 3 Full Multi-Task Report (schema=2, v22_heads=True, h_actor_dim=24)

- **Device:** `NVIDIA GeForce RTX 4060 Laptop GPU`
- **Split Sequences:** train=7450, cal=960, test=1143
- **Winner:** `3m` + `mingru` (`blake3=3a2b2161d30aad77a50d0306a6f2395de155341f4d35537bc90694745922fe56`)

| Scale | Cell | Params | T | P1 F1 | Gate F1 | ECE | TC-ECE | Unc MAE | Verify F1 | EncImp F1 | MemRel F1 | Rel Top-1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150k | mingru | 184,349 | 2.613 | 0.4472 | 0.8106 | 0.0259 | 0.0263 | 0.0108 | 0.8195 | 0.9888 | 0.9948 | 1.0000 |
| 150k | gated_deltanet2 | 192,885 | 2.629 | 0.4347 | 0.8079 | 0.0249 | 0.0257 | 0.0105 | 0.8212 | 0.9784 | 0.9948 | 1.0000 |
| 3m | mingru | 3,225,213 | 2.717 | 0.4705 | 0.8144 | 0.0226 | 0.0240 | 0.0070 | 0.8356 | 0.9935 | 0.9969 | 1.0000 |
| 1m | gated_deltanet2 | 1,271,061 | 2.777 | 0.4544 | 0.8134 | 0.0223 | 0.0237 | 0.0072 | 0.8238 | 0.9906 | 0.9969 | 1.0000 |

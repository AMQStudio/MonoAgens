# Phase 2 Multi-Task Report (`gate_4` + `wait_delta_t` + `p_uncertainty` + `p_verify`)

- **Device:** `NVIDIA GeForce RTX 4060 Laptop GPU`
- **Split Sequences (P1+P2):** train=5164, cal=755, test=849
- **Winner:** `150k` + `mingru` (`blake3=95a029c0e06ca10c60823457e90ab961455e7c757c93e76a0d13651ef75d7bbb`)

| Scale | Cell | Params | T | CIC τ | P1 F1 | Gate F1 | ECE | TC-ECE | AURC | Unc MAE | Unc Brier | Verify F1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 150k | mingru | 169,527 | 1.307 | 0.250 | 0.4965 | 0.7851 | 0.0268 | 0.0299 | 0.0128 | 0.0091 | 0.0002 | 0.9148 |
| 150k | gated_deltanet2 | 175,735 | 1.183 | 0.250 | 0.4853 | 0.7853 | 0.0319 | 0.0325 | 0.0126 | 0.0091 | 0.0002 | 0.8936 |
| 3m | mingru | 3,171,303 | 1.582 | 0.250 | 0.4882 | 0.7761 | 0.0297 | 0.0316 | 0.0134 | 0.0073 | 0.0001 | 0.9190 |
| 1m | gated_deltanet2 | 1,219,239 | 1.194 | 0.250 | 0.4574 | 0.7799 | 0.0364 | 0.0358 | 0.0129 | 0.0066 | 0.0001 | 0.9125 |

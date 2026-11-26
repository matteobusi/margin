# Proving on rented GPUs

Status: the `--cluster` client is implemented; the Bento side below follows the Boundless
repository (`prover-compose.yml`, `justfile`, November 2026) and has **not yet been run
end to end with Margin**. Benchmark one GPU first (`scripts/gpu-bench.sh`).

## When to use a cluster

`margin prove` and `margin build` prove on the local CPU by default (about 8,000 cycles/s on
16 cores). For large developments, send the jobs to a cluster of NVIDIA GPUs:

```bash
margin build --cluster http://localhost:8081 --project . <env.cache> Main main.receipt thm1 thm2
```

`margin build` sends every module whose imports are done at the same time (it defaults to
one parallel job per CPU core with `--cluster`; set `--jobs N`). The cluster splits each
job into segments and proves them on all GPUs in parallel.

**Privacy.** The cluster receives the prover input, which contains your proofs. Use it
for public developments, or on machines you trust. Rented machines are run by someone
else.

## The cluster: Bento

[Bento](https://github.com/boundless-xyz/boundless/tree/main/bento) is RISC Zero's
open-source proving cluster (the one behind the Boundless market; we only use it as a
private prover). Its REST API is compatible with the Bonsai API that `margin` speaks,
including assumption receipts, which `margin build` needs for module composition.

### Head machine (one GPU machine, or a small CPU machine)

Prerequisites: Docker with the compose plugin, the NVIDIA driver and the NVIDIA Container
Toolkit (on GPU machines), and [`just`](https://github.com/casey/just).

```bash
git clone https://github.com/boundless-xyz/boundless
cd boundless
just bento            # uses prover-compose.yml: redis, rest_api (port 8081),
                      # CPU executor agents, one GPU prove agent per local GPU
docker compose -f prover-compose.yml ps   # all services healthy
curl -f http://localhost:8081/health
```

Useful settings (environment variables read by `prover-compose.yml`):

| Variable | Default | Meaning |
|---|---|---|
| `SEGMENT_SIZE` | 21 | segment size as a power of two (2^21 cycles); 20 for GPUs with less than 16 GB |
| `BENTO_EXECUTOR_COUNT` | 4 | CPU executor agents |
| `REDIS_PASSWORD` | empty | set it if Redis is reachable from other machines |

### More GPU machines

Each extra machine runs only the GPU prove agent, pointed at the head's Redis:

```bash
git clone https://github.com/boundless-xyz/boundless && cd boundless
REDIS_HOST=<head-ip> REDIS_PASSWORD=<secret> \
  docker compose -f prover-compose.yml up -d --no-deps gpu_prove_agent
```

Open port 6379 of the head only to the workers (firewall or a private network), with a
Redis password.

### Connecting margin

Do not expose port 8081 publicly: the API has no authentication. Tunnel it:

```bash
ssh -N -L 8081:localhost:8081 user@<head-ip> &
margin build --cluster http://localhost:8081 ...
```

`margin` passes `$MARGIN_CLUSTER_KEY` (default `margin`) as the API key; Bento ignores it.

## Measured GPU speed

`scripts/gpu-bench.sh` on one rented RTX 4090 (2026-11-26, checker v0.0.1):

| Input | Cycles (padded) | 4090 | 16-core CPU (VM) |
|---|---|---|---|
| `double_comm` (core Lean) | 19.9 M | 33.6 s (594 k cycles/s) | ~40 min |
| `infinitely_many_primes` (Mathlib) | 30.5 M | 52.2 s (584 k cycles/s) | 61 min |

About 590,000 cycles per second per 4090, roughly 73 times the CPU.

## Cost estimate

Run `margin build --check-only` first: it checks every module natively and writes each
module's prover input to the cache. `margin execute <cache>/<key>/input.bin` gives the
exact cycle count of a module without proving (`--cycles` sums them for you); divide the
total by about 590,000 cycles per second per RTX 4090.

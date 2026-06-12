# CosyVoice3 FP16 Test Runbook

Date: 2026-06-10

This document records how the isolated CosyVoice3 fp16 test worker was started and benchmarked. The test did not change production code, did not modify the original `.pt` model files, and did not attach the fp16 worker to the production TTS load balancer.

## Goal

Evaluate whether the current Python/PyTorch CosyVoice3 service can run with fp16 weights on CUDA, while preserving audio quality and reducing GPU memory usage.

## Important Boundary

The fp16 test used the existing official PyTorch model files:

```text
/home/t2_enroll_ai/cosyvoice3/pretrained_models/Fun-CosyVoice3-0.5B/llm.pt
/home/t2_enroll_ai/cosyvoice3/pretrained_models/Fun-CosyVoice3-0.5B/flow.pt
/home/t2_enroll_ai/cosyvoice3/pretrained_models/Fun-CosyVoice3-0.5B/hift.pt
```

The files above remained fp32 on disk. The test worker converted the loaded modules to fp16 in memory only:

```python
server.cosyvoice.model.llm.half()
server.cosyvoice.model.flow.half()
server.cosyvoice.model.hift.half()
```

This is different from the GGUF F16/Q8 test. GGUF files cannot be loaded by the current Python `AutoModel` runtime. They require a different runtime such as CrispASR, and the tested CrispASR `cosyvoice3-tts` path was pinned to CPU, so it was not suitable for production.

## Production Baseline

Production TTS workers:

```text
50000  model-service-stack load balancer
50001  fp32 CosyVoice3 worker, GPU 8
50002  fp32 CosyVoice3 worker, GPU 9
50003  fp32 CosyVoice3 worker, GPU 8
50004  fp32 CosyVoice3 worker, GPU 9
```

The fp16 test worker:

```text
50005  fp16 CosyVoice3 test worker, GPU 7
```

It was not added to `50000`, so normal site traffic continued to use the existing fp32 production workers.

## Environment

Verified server environment:

```text
Host: t2_enroll_ai@10.10.200.13
CosyVoice directory: /home/t2_enroll_ai/cosyvoice3
Python: 3.10.4
Torch: 2.12.0+cu126
torch.version.cuda: 12.6
CUDA available: true
```

GPU allocation at test time:

```text
GPU 7: mostly free, also had one embedding worker
GPU 8/9: production CosyVoice3 workers
```

## Startup Script

The test worker was started from a temporary script:

```bash
cat > /tmp/cosy_fp16_server_50005.py <<'PY'
import gc
import importlib.util
import logging
import sys
import torch
import uvicorn

ROOT = "/home/t2_enroll_ai/cosyvoice3"
MODEL = ROOT + "/pretrained_models/Fun-CosyVoice3-0.5B"
SERVER = ROOT + "/runtime/python/fastapi/server.py"

sys.path.append(ROOT)
sys.path.append(ROOT + "/third_party/Matcha-TTS")
spec = importlib.util.spec_from_file_location("cosyvoice_fastapi_server_fp16", SERVER)
server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(server)

server.cosyvoice = server.AutoModel(model_dir=MODEL, fp16=True)
server.cosyvoice.fp16 = True
server.cosyvoice.model.fp16 = True
server.cosyvoice.model.llm.half()
server.cosyvoice.model.flow.half()
server.cosyvoice.model.hift.half()

gc.collect()
torch.cuda.empty_cache()
torch.cuda.synchronize()

logging.warning(
    "fp16 test worker ready: sample_rate=%s dtypes llm=%s flow=%s hift=%s allocated_mib=%.1f reserved_mib=%.1f",
    server.cosyvoice.sample_rate,
    next(server.cosyvoice.model.llm.parameters()).dtype,
    next(server.cosyvoice.model.flow.parameters()).dtype,
    next(server.cosyvoice.model.hift.parameters()).dtype,
    torch.cuda.memory_allocated() / 1024 / 1024,
    torch.cuda.memory_reserved() / 1024 / 1024,
)
uvicorn.run(server.app, host="0.0.0.0", port=50005)
PY

cd /home/t2_enroll_ai/cosyvoice3
CUDA_VISIBLE_DEVICES=7 nohup ./.venv/bin/python /tmp/cosy_fp16_server_50005.py \
  > /home/t2_enroll_ai/model-service-logs/cosyvoice-fp16-50005.log 2>&1 &
```

Log file:

```text
/home/t2_enroll_ai/model-service-logs/cosyvoice-fp16-50005.log
```

Expected readiness log:

```text
fp16 test worker ready: sample_rate=24000 dtypes llm=torch.float16 flow=torch.float16 hift=torch.float16
Uvicorn running on http://0.0.0.0:50005
```

## Verification Commands

Check port:

```bash
ss -ltnp | grep ':50005'
```

Check GPU memory:

```bash
nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv,noheader,nounits
```

The fp16 worker process used about `2466MiB`, compared with about `4036MiB` for each fp32 production worker.

## Audio Generation Test

Short text comparison:

```bash
TEXT="哈尔滨师范大学欢迎各位考生和家长咨询招生政策、专业培养方案和近年录取数据。"

/usr/bin/time -f "FP16_WALL=%e" \
  curl -sS -X POST http://127.0.0.1:50005/tts_stream \
  -H "Content-Type: application/json" \
  --data "{\"text\":\"$TEXT\"}" \
  -o /tmp/cosy_fp16_test.pcm

/usr/bin/time -f "FP32_WALL=%e" \
  curl -sS -X POST http://127.0.0.1:50001/tts_stream \
  -H "Content-Type: application/json" \
  --data "{\"text\":\"$TEXT\"}" \
  -o /tmp/cosy_fp32_test.pcm
```

Long text comparison:

```bash
TEXT="如果你想了解哈尔滨师范大学某个专业的录取概率，建议同时提供省份、科类、分数、位次和目标专业。我们可以结合近年录取数据、招生计划和培养方案，帮你做更稳妥的参考判断。"

/usr/bin/time -f "FP16_LONG_WALL=%e" \
  curl -sS -X POST http://127.0.0.1:50005/tts_stream \
  -H "Content-Type: application/json" \
  --data "{\"text\":\"$TEXT\"}" \
  -o /tmp/cosy_fp16_long.pcm

/usr/bin/time -f "FP32_LONG_WALL=%e" \
  curl -sS -X POST http://127.0.0.1:50001/tts_stream \
  -H "Content-Type: application/json" \
  --data "{\"text\":\"$TEXT\"}" \
  -o /tmp/cosy_fp32_long.pcm
```

Convert raw PCM to WAV:

```bash
/home/t2_enroll_ai/cosyvoice3/.venv/bin/python - <<'PY'
import numpy as np
import soundfile as sf

for stem in ["fp16", "fp32", "fp16_long", "fp32_long"]:
    pcm = f"/tmp/cosy_{stem}_test.pcm" if stem in ["fp16", "fp32"] else f"/tmp/cosy_{stem}.pcm"
    wav = f"/tmp/cosy_{stem}_test.wav" if stem in ["fp16", "fp32"] else f"/tmp/cosy_{stem}.wav"
    data = np.fromfile(pcm, dtype=np.int16)
    sf.write(wav, data.astype(np.float32) / 32768.0, 24000, format="WAV", subtype="PCM_16")
    print(stem, "samples", data.size, "duration", round(data.size / 24000, 3), "wav", wav)
PY
```

The generated WAV files were copied locally to:

```text
/home/scm2002/Code/rust_enrollment/tmp/cosyvoice_fp16_worker/
```

Files:

```text
cosy_fp16_test.wav
cosy_fp32_test.wav
cosy_fp16_long.wav
cosy_fp32_long.wav
```

## Results

Observed results:

```text
fp16 worker memory: ~2466MiB
fp32 worker memory: ~4036MiB
```

Timing:

```text
Short text:
  fp16: 6.93s
  fp32: 5.76s

Long text:
  fp16: 8.34s
  fp32: 8.31s
```

Audio durations:

```text
cosy_fp16_test.wav   7.60s
cosy_fp32_test.wav   8.36s
cosy_fp16_long.wav  15.08s
cosy_fp32_long.wav  14.20s
```

User listening result:

```text
Audio quality was considered good.
```

## Interpretation

The fp16 PyTorch CUDA path is useful for reducing GPU memory. In this isolated test, memory dropped from roughly `4.0GB` per worker to roughly `2.5GB` per worker.

Speed was not meaningfully better. Short text was slightly slower in fp16, while longer text was essentially the same. The main benefit is memory reduction and the possibility of running more workers per GPU if concurrency requires it.

## Rollback / Stop Test Worker

The fp16 worker is safe to stop because it is not part of the production load balancer.

Find the process:

```bash
ss -ltnp | grep ':50005'
ps -eo pid,cmd | grep cosy_fp16_server_50005 | grep -v grep
```

Stop it:

```bash
kill <pid>
```

If needed:

```bash
kill -9 <pid>
```

Removing the temporary script is optional:

```bash
rm -f /tmp/cosy_fp16_server_50005.py
```

## Next Step Recommendation

Keep `50005` as a private fp16 test worker for more manual listening and limited concurrency tests. Do not replace all production workers at once.

If it stays stable, the next safe rollout would be:

1. Keep two fp32 workers unchanged.
2. Start one or two fp16 workers on separate ports.
3. Add them to the TTS load balancer with low traffic weight or manual test routing.
4. Observe first audio latency, interruptions, GPU memory, and audio quality.
5. Expand only if no regressions appear.

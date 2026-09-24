# Gates: M12 Recovery hardening

OWNS: crates/tachyon-store/**, crates/tachyon-core/**, crates/tachyon-gateway/**, crates/tachyon-mutation/**, crates/tachyon-verify/**, docs/milestones/M12_REPORT.md, PROGRESS.md, GATES.md

Scope: Ship issue #8 — env-gated fault points, effect fixture + recover_task §19 reconcile, driver re-entry with fresh-id re-ask, kill tests across six domains, gateway SIGKILL test, §42 matrix report, PROGRESS entry, follow-up issue.

- [x] G1: Workspace fmt/check/clippy green
  CHECK: cargo fmt --check && cargo check --workspace && echo fmt_check_ok
  EXPECT: fmt_check_ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=3039ca5253419206113019a72fa4ea99361f57d81c1762ed66fc6fd418454650; exit=0; EXPECT=matched; output-sha256=aee819a862a12d350cee2a4beebfce79d58499746522b86b885b4a9a4c053621; output-bytes=86; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G2: Full workspace tests green (includes all new M12 tests)
  CHECK: cargo test --workspace 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=a95c7026bb8b5b13a0a38c49fb4bfdbc2e5f5caa13b592abaff620ec82b2faf4; exit=0; EXPECT=matched; output-sha256=dca17eb542bd7b4c09b0f7b8a8a2a81be3e279e5692a39e56865cc81d9cc33fa; output-bytes=49174; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G3: Strict clippy green
  CHECK: cargo clippy --workspace --all-targets -- -D warnings 2>&1
  EXPECT: Finished
  EVIDENCE: automatic-evidence=v1; definition-sha256=32178d57bd94e58f59aeda517a509d56caabac3ed2edfb4c640e4ff8ff69856b; exit=0; EXPECT=matched; output-sha256=43f65fba4cf9728ccc92b3fbdcfa80e0849b2d5ae2f4c61901ceed3cdb0a3a63; output-bytes=127; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G4: Fault-point helper exists and is env-gated (unit: no arm = no block)
  CHECK: cargo test -p tachyon-tools fault_point 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=f320dbe630d770999ae220e9b639cdd49e536b6563bfc0e4e5ea139186e3f6c0; exit=0; EXPECT=matched; output-sha256=56ecaf15d03719831cbbd80120883cb0be3f0f11d0fc01d4f7257cb083583a61; output-bytes=1764; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G5: Effect fixture §19 reconcile tests pass
  CHECK: cargo test -p tachyon-core effect_fixture 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=a019e9f51bcb6d522b9c2720dc7ec164cf079948d41705c51af286249b1d87a6; exit=0; EXPECT=matched; output-sha256=bd36548cf95099afdbafe05304ec3e3f16dd388df0b16bbc762b12e2ae618f66; output-bytes=3891; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G6: Driver re-entry / fresh-id tests pass
  CHECK: cargo test -p tachyon-gateway reentry 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=111a49d225026564cdcfa439c096c6d8584c226a41f1b05569fd4a3a22b9fd35; exit=0; EXPECT=matched; output-sha256=7a6e83ea83d0ba6ebecf3f51f88cfc92af50a670d48aae4be5ebe27e7525e9fb; output-bytes=2527; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G7: Gateway SIGKILL restart test passes
  CHECK: cargo test -p tachyon-app kill_restart 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=20ffc3b0f9405902fe7548898778b30f6bbd6e0b5b088de768caf0e3c3d65b22; exit=0; EXPECT=matched; output-sha256=971f540c711755b20e217b7508170f17825fe52de2f6f5ce63b518522c43919c; output-bytes=1039; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G8: Six-domain kill/gate suites exist and pass (native reads, model, mutation, verifier, approval, effect)
  CHECK: cargo test --workspace gates 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=36a855514f02f60e21978e0f144e93f1cdfa44438211f0ede99b7d6acc2e074d; exit=0; EXPECT=matched; output-sha256=a2fe47f92ec5661a0073d6525dae2ae13a9365da3ebe2b0fc805249bd1dd82bb; output-bytes=16110; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G8b: Armed fault-point child parks and is SIGKILLed
  CHECK: cargo test -p tachyon-core --test fault_kill 2>&1
  EXPECT: test result: ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=6514fd13d5fdd314886f0a6a9bf37005d108c362bbe6a8023ce9a2117baf9dae; exit=0; EXPECT=matched; output-sha256=dcabb7d727481f9059b53a6eac441c9a1815477ac1783dd0929ef39b2700e9cb; output-bytes=542; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G9: M12_REPORT.md exists with §42 matrix covering all 7 points
  CHECK: node -e "const fs=require('fs');const t=fs.readFileSync('docs/milestones/M12_REPORT.md','utf8');const pts=['journal commit','EffectPrepared','remote effect return','EffectCommitted','multi-file mutation','verification','approval wait'];const miss=pts.filter(p=>!t.toLowerCase().includes(p.toLowerCase()));if(miss.length){console.error('missing',miss);process.exit(1)};console.log('matrix ok')"
  EXPECT: matrix ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=4a3b5bcee00b27badcd94ff86fb8ff3033f16f74350a4e7b603baa4ae912c67a; exit=0; EXPECT=matched; output-sha256=1b44bebea7866786f89de56ffa7ac782c15df61676ce89ddd0944c9f30c293cd; output-bytes=10; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G10: PROGRESS.md has Milestone 12 completed-gates entry
  CHECK: node -e "const t=require('fs').readFileSync('PROGRESS.md','utf8');if(!/Milestone 12/.test(t)){process.exit(1)};console.log('progress ok')"
  EXPECT: progress ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=a0c10feb5d0d43f7e7034ae7721f7dc72087305995faf21c53aa5504fb465a07; exit=0; EXPECT=matched; output-sha256=e500d7c1ac8f12963e6ab03611c408e9d733a01734b2486d268517781e38d974; output-bytes=12; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

- [x] G11: ADRs 0001 and 0002 present
  CHECK: node -e "const fs=require('fs');for(const f of ['docs/adr/0001-env-gated-fault-points.md','docs/adr/0002-driver-re-entry-is-user-triggered.md']){if(!fs.existsSync(f))process.exit(1)};console.log('adrs ok')"
  EXPECT: adrs ok
  EVIDENCE: automatic-evidence=v1; definition-sha256=eac7b74212f3eed7fb253b884904069b8bacedcf267ac09acb89f5261e5a218d; exit=0; EXPECT=matched; output-sha256=e827697627329eff6278e7ee35f228c2b14f5613c7b09b9eec46201ea71e7015; output-bytes=8; shell=/bin/sh; cwd=/run/media/its1deat0r/Projects/AI Agents/Tachyon Agent; path=022ddecf2f49/16 entries

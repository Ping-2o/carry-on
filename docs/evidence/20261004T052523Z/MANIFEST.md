# Carry-On cross-device evidence archive

- commit: `e5abb4ca8a88ac77969e036c5184b23bc4466027`
- collected: 2026-10-04T05:28:05Z
- source: mac (192.168.3.201) — see `mac.json`
- destination: android device — see `device.json`

## Contents
- `campaign.json`
- `checksums.sha256`
- `commit.txt`
- `device.json`
- `device.png`
- `fault-report.json`
- `mac.json`
- `pcap-command.txt`
- `RESULTS-xdevice.md`
- `source-off-proof.json`
- `src-t_A_1048576_0_0_p48986.log`
- `src-t_A_1048576_0_0_p48987.log`
- `src-t_A_1048576_1024_20_p48974.log`
- `src-t_A_1048576_600000_0_p48983.log`
- `src-t_A_256_0_0_p48996.log`
- `src-t_A_4096_600000_0_p48977.log`
- `src-t_A_65536_0_0_p48984.log`
- `src-t_A_65536_600000_20_p48995.log`
- `src-t_A_65536_600000_5_p48998.log`
- `src-t_C_1048576_0_20_p48993.log`
- `src-t_C_1048576_0_5_p48971.log`
- `src-t_C_1048576_1024_5_p48999.log`
- `src-t_C_1048576_600000_0_p48972.log`
- `src-t_C_256_1024_0_p48991.log`
- `src-t_C_256_600000_0_p48988.log`
- `src-t_C_4096_1024_0_p48997.log`
- `src-t_C_4096_600000_0_p48989.log`
- `src-t_C_65536_1024_5_p48979.log`
- `src-t_C_65536_65536_20_p48990.log`
- `src-t_D_1048576_65536_0_p48980.log`
- `src-t_D_1048576_65536_20_p48973.log`
- `src-t_D_256_0_0_p48985.log`
- `src-t_D_256_0_20_p48976.log`
- `src-t_D_256_1024_0_p48992.log`
- `src-t_D_256_600000_20_p48975.log`
- `src-t_D_256_65536_20_p48970.log`
- `src-t_D_4096_0_0_p48981.log`
- `src-t_D_65536_0_5_p48982.log`
- `src-t_D_65536_600000_0_p48994.log`
- `src-t_D_65536_65536_0_p48978.log`
- `summary-xdevice.json`
- `timestamps.txt`
- `trials.jsonl`
- `uc-check.log`
- `uc-dst.log`
- `uc-src.log`
- `unsaved-continue-evidence.json`

## Disclosure
PHYSICAL cross-device evidence (PLAT-001): two machines, two NICs, real TLS 1.3 + mutual
cert pinning over the LAN. The packet capture shows the TLS record layer; payload is
encrypted (expected). Still no APK/signing and no §30 platform acceptance (spec §2/§30).
Loopback-within-one-device anywhere in the repo remains LOCAL evidence, device or not.

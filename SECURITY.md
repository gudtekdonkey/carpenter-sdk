# Security

Please report a vulnerability privately, through GitHub's private vulnerability reporting on this repository
(**Security** → **Report a vulnerability**), not in a public issue. Include what is affected (package and version, or
commit), how to reproduce it, and its impact.

You will get an acknowledgement within three working days. Please give us reasonable time to fix the issue before
disclosing it.

This repository holds SDKs: libraries that read on-chain state, quote trades and build unsigned transactions. A quote or a
transaction built here is checked again by the programs and contracts it targets, and by the wallet that signs it.

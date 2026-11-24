# Phase 15 — Migration ও Import (v2)

### T15.01 — Import framework
- **কী করতে হবে:** `ImportSource` trait: archive parse করা → একটা সাধারণ intermediate model (account, domain, DB, mail, file, cron, DNS)।
- **DoD:** Intermediate model-এর schema ও test আছে।

### T15.02 — cPanel backup (cpmove) parser
- **DoD:** একটা নমুনা cpmove ফাইল থেকে সঠিক model তৈরি হয়।

### T15.03 — CyberPanel ও OpenPanel backup parser
- **DoD:** একই ভাবে।

### T15.04 — Import executor
- **কী করতে হবে:** Model → Built Panel tenant, domain, DB, file (operation chain)। Dry-run (কী হবে তা দেখানো)। Conflict সমাধান (username বা domain আগে থেকে আছে)।
- **DoD:** Dry-run রিপোর্ট সঠিক। আসল import-এর পর site চলে।

### T15.05 — Remote fetch
- **কী করতে হবে:** SSH বা API দিয়ে উৎস server থেকে backup আনা (credential এক-বার ব্যবহার করে মুছে ফেলা)।
- **DoD:** Credential DB-তে থাকে না।

### T15.06 — Transfer অবস্থা ও log UI
- **DoD:** OpenPanel-এর `/import/transfers`-এর মতো: চলমান অবস্থা, ধাপ অনুযায়ী log।

### T15.07 — Tenant transfer (node থেকে node)
- **কী করতে হবে:** Export → নতুন node-এ import → DNS বা IP পরিবর্তন → পুরনো node পরিষ্কার। Downtime কমাতে আগে rsync, তারপর শেষ sync।
- **DoD:** ১০GB tenant-এর downtime ২ মিনিটের কম।

---
"release": patch
---

# SQLite live queries hear other processes' writes

On SQLite, a live query now updates within a quarter of a second when another process writes to the same database file, such as a second server process. Before, it heard only its own process's writes, and saw another process's writes only once it wrote something itself. Each database file gets one connection that checks for changes four times a second, and only once something subscribes to changes. Writes from tools that aren't leptoskit apps, such as the `sqlite3` shell, still reach subscribers with the app's next write.

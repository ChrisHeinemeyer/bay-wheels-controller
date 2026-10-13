# Home Assistant add-on: Bay Wheels Log

Receives the JSON status report the firmware POSTs once per fetch cycle when
built with the `log-server` feature, keeps it in the add-on's `/data`, and
serves a status page.

- `POST /report`: one report (sent by the firmware)
- `GET /`: status page (auto-refreshes every 30s), at `http://10.0.0.107:8099/`
- `GET /api/reports?limit=N`: raw reports as JSON

Deploy or update (copies over SSH, then installs or rebuilds on the device):

```
homeassistant/deploy.sh                 # default target root@homeassistant.local
```

Bump `version` in `bay_wheels_log/config.yaml` when you change the add-on.

Firmware side: set `LOG_SERVER="10.0.0.107:8099"` in `.env` and build with
`--features use-env,light-sleep,log-server`.

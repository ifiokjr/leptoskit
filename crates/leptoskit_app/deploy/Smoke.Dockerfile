# The candidate must run without the build runner, Cargo, Nix or network access.
FROM ubuntu:24.04
RUN apt-get update \
 && apt-get install --yes --no-install-recommends ca-certificates libstdc++6 python3 \
 && rm -rf /var/lib/apt/lists/*
COPY bundle /bundle
COPY checks /checks
ENV PYTHONDONTWRITEBYTECODE=1
USER 10001:10001
ENTRYPOINT ["python3", "/checks/hosted_pilot_local.py", "--binary", "/bundle/leptoskit-cloud", "--site", "/bundle/site"]

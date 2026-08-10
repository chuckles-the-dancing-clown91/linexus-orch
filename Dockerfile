# Linexus Orchestrator — demand/supply engine + DAG planner.
FROM rust:1-slim AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config build-essential ca-certificates git && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
# linexus-core arrives as a git dependency — git + network needed at build time.
RUN cargo build --release --bin linexus-orch

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 linexus
COPY --from=build /app/target/release/linexus-orch /usr/local/bin/linexus-orch
USER linexus
ENV ORCH_BIND=0.0.0.0:5152
EXPOSE 5152
CMD ["linexus-orch"]

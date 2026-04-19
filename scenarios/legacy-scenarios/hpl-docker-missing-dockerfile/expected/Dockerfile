FROM ubuntu:22.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y \
    libopenmpi-dev \
    openmpi-bin \
    libopenblas-dev \
    liblapack-dev \
    hpcc \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /hpl
COPY run_hpl.sh .
RUN chmod +x run_hpl.sh
CMD ["./run_hpl.sh"]

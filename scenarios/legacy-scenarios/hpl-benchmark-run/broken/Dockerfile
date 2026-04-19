FROM ubuntu:22.04
RUN apt-get update && apt-get install -y \
    libopenmpi-dev \
    openmpi-bin \
    libblas-dev \
    liblapack-dev \
    hpl-benchmark-pkg
WORKDIR /hpl
COPY run_hpl.sh .
RUN chmod +x run_hpl.sh
CMD ["./run_hpl.sh"]

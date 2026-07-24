#!/usr/bin/env bash

docker build . -t jolt_test_server && docker run jolt_test_server:latest


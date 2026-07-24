#!/usr/bin/env bash

zig build run -Dllvm=false -Dno-bin -freference-trace

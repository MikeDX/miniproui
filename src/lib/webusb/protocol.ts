// Copyright (C) 2026 MikeDX https://github.com/MikeDX/miniui
// Protocol based on minipro, created by Valentin Dudouyt in 2014.
// Development of that C project is coordinated by David Griffith.
// SPDX-License-Identifier: GPL-3.0-or-later

export const HEADER_WORDS = 16;
export const DATA_OFFSET = 64;
export const MAX_BYTES = 16 * 1024 * 1024;
export const SAB_SIZE = DATA_OFFSET + MAX_BYTES;

export const STATE_IDLE = 0;
export const STATE_REQ = 1;
export const STATE_RESP = 2;

export const OP_OUT = 1;
export const OP_IN = 2;
export const OP_OUT_PAIR = 3;
export const OP_IN_PAIR = 4;

export const H_STATE = 0;
export const H_OP = 1;
export const H_EP_A = 2;
export const H_EP_B = 3;
export const H_LEN_A = 4;
export const H_LEN_B = 5;
export const H_STATUS = 6;
export const H_ACTUAL = 7;
export const H_ACTUAL_B = 8;

export const SEND_TIMEOUT_MS = 5_000;
export const RECV_TIMEOUT_MS = 360_000;

export const PROGRAMMER_FILTERS: USBDeviceFilter[] = [
  { vendorId: 0x04d8, productId: 0xe11c },
  { vendorId: 0xa466, productId: 0x0a53 },
  { vendorId: 0xa466, productId: 0x1a86 },
];

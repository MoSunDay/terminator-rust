#!/usr/bin/env python3
"""Private PTY fixture: delayed commit echo, cursor moves, wrap and hiding."""
import os
import termios
import time
import tty

saved = termios.tcgetattr(0)
prefix = '汉字'.encode()


def draw(position):
    cols, rows = os.get_terminal_size(0)
    row, col = {'top': (3, 2), 'bottom': (rows, max(1, cols - 8)),
                'wrap': (rows, max(1, cols - 2)), 'edge': (3, max(1, cols - 3)),
                'wide': (3, max(1, cols - 4)), 'last': (rows, max(1, cols - 3))}[position]
    os.write(1, f'\x1b[2J\x1b[HDELAY_READY\x1b[{row};{col}H\x1b[91m'.encode()
             + prefix + b'\x1b[0m')


try:
    tty.setraw(0)
    os.write(1, b'\x1b[2J\x1b[HDELAY_READY\x1b[3;2H')
    data = b''
    while len(data) < len(prefix):
        data += os.read(0, 4096)
    if data != prefix:
        raise RuntimeError('unexpected first commit: ' + repr(data))
    time.sleep(1.2)
    os.write(1, b'\x1b[91m' + data + b'\x1b[0m')
    while True:
        data = os.read(0, 4096)
        if data == b'q':
            break
        if data in (b't', b'b', b'w', b'e', b'i', b'l'):
            draw({b't': 'top', b'b': 'bottom', b'w': 'wrap',
                  b'e': 'edge', b'i': 'wide', b'l': 'last'}[data])
        elif data in (b'h', b'v'):
            os.write(1, b'\x1b[?25' + (b'l' if data == b'h' else b'h'))
        else:
            os.write(1, data)
finally:
    os.write(1, b'\x1b[?25h\x1b[0m\r\n')
    termios.tcsetattr(0, termios.TCSANOW, saved)

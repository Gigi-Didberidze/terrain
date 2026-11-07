import sys

from shop import checkout


def main():
    print(checkout([float(a) for a in sys.argv[1:]]))


if __name__ == "__main__":
    main()

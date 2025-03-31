import csv
import glob
import matplotlib.pyplot as plt
import time

MARKERS = [
    "^",
    "o",
    "s",
    "d",
    "v",
    "P",
    ">",
    "X",
    "<",
    "*",
]


def read_csv(file_name):
    with open(file_name) as csv_file:
        csv_reader = csv.DictReader(csv_file)

        rows = []
        for csv_line in csv_reader:
            try:
                row = {
                    "clients": int(csv_line["clients"]),
                    "latency": float(csv_line["latency"]),
                    "init_conn_time": float(csv_line["init_conn_time"]),
                    "tps": float(csv_line["tps"]),
                }
                rows.append(row)
            except:
                print("Unable to parse row from csv_line:", csv_line)

        return rows


def main():
    fig, ax = plt.subplots(figsize=(16, 12), layout="constrained")

    files = [f for f in glob.glob("results/*.csv")]
    files = sorted(files)

    for i, file in enumerate(files):
        label = file.replace(".csv", "")
        data = read_csv(file)

        clients = [d["clients"] for d in data]
        tps = [d["tps"] for d in data]
        ax.plot(
            clients,
            tps,
            label=label,
            linestyle="-",
            marker=MARKERS[i],
            markeredgewidth=1,
            markersize=10,
        )

    # Ensure the baseline starts at zero
    ax.set_xlim(left=0)  # Set x-axis lower limit to 0
    ax.set_ylim(bottom=0)  # Set y-axis lower limit to 0


    fig.legend(loc="outside upper left")
    plt.xlabel("clients")
    plt.ylabel("tps")
    plt.title("Transactions per second")

    ts = time.strftime("%Y%m%d%H%M")
    file_name = "benchmark-{}.png".format(ts)
    plt.savefig(file_name)


if __name__ == "__main__":
    main()

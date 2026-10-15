r"""Writes rust/crates/sf-core/tests/reference_graph.txt:

  * the Zachary karate-club graph with networkx's Louvain modularity (a classic community-detection benchmark);
  * 1000 cells x 6 markers with the graph and clusters the Python engine's `phenograph` package produces
    (k=30, Jaccard, Louvain): its lower-triangle edge list, its modularity Q, and its labels.

The Rust test rebuilds the PhenoGraph graph from the same cells and requires every edge weight to match.

    engine\py-env\Scripts\python.exe rust\tools\gen_graph_reference.py
"""
import os
import numpy as np
import networkx as nx
import phenograph

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "sf-core", "tests", "reference_graph.txt")
SRC = os.path.join(os.path.dirname(__file__), "..", "crates", "sf-core", "tests", "reference_cluster.txt")


def main():
    
    lines = []
    G = nx.karate_club_graph()
    qs = []
    for seed in range(20):
        comms = nx.community.louvain_communities(G, weight=None, seed=seed)
        qs.append(nx.community.modularity(G, comms, weight=None))
    lines.append("karate_q_best %r" % max(qs))
    lines.append("karate_q_mean %r" % float(np.mean(qs)))
    lines.append("karate_edges %d" % G.number_of_edges())
    for a, b in G.edges():
        lines.append("e %d %d" % (a, b))
    
    rows = [l.split() for l in open(SRC).read().splitlines()[3:4000]]
    d = len(rows[0]) - 3
    X = np.array([[float(v) for v in r[:d]] for r in rows])[:1000]
    Z = (X - X.mean(0)) / X.std(0)
    com, graph, Q = phenograph.cluster(Z, k=30, n_jobs=1, clustering_algo="louvain", min_cluster_size=10)
    g = graph.tocoo()
    lines.append("cells %d %d" % Z.shape)
    lines.append("pheno_q %r" % float(Q))
    lines.append("pheno_nclusters %d" % len(set(com.tolist()) - {-1}))
    lines.append("pheno_edges %d" % g.nnz)
    for i, j, w in zip(g.row, g.col, g.data):
        lines.append("g %d %d %r" % (i, j, float(w)))
    for r, c in zip(Z, com):
        lines.append("c " + " ".join(repr(float(v)) for v in r) + " %d" % c)
    
    with open(OUT, "w", newline="\n") as fh:
        fh.write("\n".join(lines) + "\n")
    print("karate louvain Q best %.4f mean %.4f" % (max(qs), np.mean(qs)))
    print("phenograph Q %.4f, clusters %d, edges %d" % (Q, len(set(com.tolist()) - {-1}), g.nnz))
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()

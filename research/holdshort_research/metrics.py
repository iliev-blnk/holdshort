"""Error metrics, in the units of the target."""

import numpy as np


def summary(y_true, y_pred) -> dict:
    err = np.abs(np.asarray(y_pred, dtype=float) - np.asarray(y_true, dtype=float))
    return {
        "n": int(err.size),
        "mae": float(err.mean()),
        "rmse": float(np.sqrt((err**2).mean())),
        "p90": float(np.quantile(err, 0.9)),
    }

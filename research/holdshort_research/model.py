"""LightGBM training and ONNX export, shared by both targets.

Training is two steps: early stopping on the validation days picks the number of
trees, then the final model is refit with exactly that many trees. The final model
then has no early-stopping state, so its ONNX export (which writes every tree) gives
the same predictions as LightGBM.
"""

from pathlib import Path

import lightgbm as lgb
import numpy as np
import onnxruntime as ort
from onnxmltools import convert_lightgbm
from onnxmltools.convert.common.data_types import FloatTensorType

PARAMS = dict(learning_rate=0.03, num_leaves=31, min_child_samples=30, subsample=0.8, subsample_freq=1, colsample_bytree=0.9, verbose=-1)


def best_trees(X, y, X_val, y_val, **params) -> int:
    """Number of trees that minimises the validation L1 error."""
    m = lgb.LGBMRegressor(n_estimators=3000, **{**PARAMS, **params})
    m.fit(X, y, eval_X=(X_val,), eval_y=(y_val,), eval_metric="l1", callbacks=[lgb.early_stopping(150, verbose=False)])
    return max(int(m.best_iteration_ or m.n_estimators), 1)


def refit(X, y, n_trees: int, **params) -> lgb.LGBMRegressor:
    return lgb.LGBMRegressor(n_estimators=n_trees, random_state=0, **{**PARAMS, **params}).fit(X, y)


def fit(X, y, X_val, y_val, **params) -> lgb.LGBMRegressor:
    """Pick the tree count on validation, then refit on train only (used by tests)."""
    return refit(X, y, best_trees(X, y, X_val, y_val, **params), **params)


def export_onnx(model: lgb.LGBMRegressor, n_features: int, path: Path) -> None:
    onx = convert_lightgbm(model, initial_types=[("input", FloatTensorType([None, n_features]))], target_opset=15)
    Path(path).write_bytes(onx.SerializeToString())


def onnx_predict(path: Path, X) -> np.ndarray:
    s = ort.InferenceSession(str(path))
    return s.run(None, {"input": np.asarray(X, dtype=np.float32)})[0].ravel()

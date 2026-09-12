#include <pxr/usd/usd/stage.h>
#include <pxr/usd/usd/primRange.h>
#include <pxr/usd/usdGeom/mesh.h>
#include <pxr/usd/usdGeom/xformCache.h>
#include <pxr/usd/usdSkel/bakeSkinning.h>
#include <pxr/base/gf/interval.h>
#include <cmath>
#include <iomanip>
#include <iostream>
#include <stdexcept>

PXR_NAMESPACE_USING_DIRECTIVE

int main(int argc, char **argv) try {
    if (argc != 4 && argc != 5) throw std::runtime_error("usage: sample_native_deformation ASSET /MESH TIME [--normals]");
    const bool sampleNormals = argc == 5;
    if (sampleNormals && std::string(argv[4]) != "--normals") throw std::runtime_error("unknown option");
    size_t consumed = 0;
    double time = std::stod(argv[3], &consumed);
    if (consumed != std::string(argv[3]).size() || !std::isfinite(time))
        throw std::runtime_error("time must be finite");
    auto source = UsdStage::Open(argv[1]);
    if (!source) throw std::runtime_error("cannot open source");
    auto stage = UsdStage::Open(source->Flatten());
    if (!stage) throw std::runtime_error("cannot open anonymous snapshot");
    auto original = source->GetPrimAtPath(SdfPath(argv[2]));
    if (!UsdGeomMesh(original)) throw std::runtime_error("requested prim is not a mesh");
    UsdGeomXformCache before{UsdTimeCode(time)};
    const auto originalWorld = before.GetLocalToWorldTransform(original);
    if (std::abs(originalWorld.GetDeterminant()) < 1e-12)
        throw std::runtime_error("original mesh transform is singular");
    for (auto const &prim : stage->Traverse()) {
        for (auto const &attribute : prim.GetAttributes()) {
            if (attribute.GetNumTimeSamples() == 0) continue;
            VtValue value;
            if (!attribute.Get(&value, UsdTimeCode(time)) || !attribute.Set(value, UsdTimeCode(time)))
                throw std::runtime_error("cannot materialize requested time sample");
        }
    }
    if (!UsdSkelBakeSkinning(UsdPrimRange::Stage(stage), GfInterval(time, time)))
        throw std::runtime_error("native skinning bake failed");
    UsdGeomMesh mesh(stage->GetPrimAtPath(SdfPath(argv[2])));
    VtVec3fArray points;
    if (!mesh || !(sampleNormals ? mesh.GetNormalsAttr() : mesh.GetPointsAttr()).Get(&points, UsdTimeCode(time)) || points.empty())
        throw std::runtime_error("requested mesh has no sampled values");
    UsdGeomXformCache after{UsdTimeCode(time)};
    const auto toOriginal = after.GetLocalToWorldTransform(mesh.GetPrim()) * originalWorld.GetInverse();
    if (sampleNormals && std::abs(toOriginal.GetDeterminant()) < 1e-12)
        throw std::runtime_error("baked normal transform is singular");
    const auto normalTransform = sampleNormals ? toOriginal.GetInverse().GetTranspose() : GfMatrix4d(1);
    std::cout << std::setprecision(9) << "time=" << time << (sampleNormals ? " normals=" : " points=") << points.size() << '\n';
    for (size_t i = 0; i < points.size(); ++i) {
        auto point = sampleNormals ? normalTransform.TransformDir(GfVec3d(points[i])) : toOriginal.Transform(GfVec3d(points[i]));
        if (sampleNormals) {
            if (point.GetLength() < 1e-12) throw std::runtime_error("zero baked normal");
            point.Normalize();
        }
        for (int axis = 0; axis < 3; ++axis)
            if (!std::isfinite(point[axis])) throw std::runtime_error("nonfinite baked point");
        std::cout << i << ' ' << point[0] << ' ' << point[1] << ' ' << point[2] << '\n';
    }
    return 0;
} catch (std::exception const &error) {
    std::cerr << error.what() << '\n';
    return 2;
}

#include <opensubdiv/far/topologyDescriptor.h>
#include <opensubdiv/far/primvarRefiner.h>
#include <algorithm>
#include <cmath>
#include <fstream>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <map>
#include <memory>
#include <set>
#include <sstream>
#include <stdexcept>
#include <vector>
using namespace OpenSubdiv;
struct Point {
    double p[3]{};
    void Clear() { for (auto &x:p) x=0; }
    void AddWithWeight(Point const &q, double w) { for(int i=0;i<3;++i) p[i]+=q.p[i]*w; }
};
std::vector<double> array(std::string const &path, std::string const &name) {
    std::ifstream in(path); if(!in) throw std::runtime_error("missing input");
    std::string line;
    while(std::getline(in,line)) if(line.find(name+" = [")!=std::string::npos) {
        auto start=line.find(" = [")+4; auto end=line.find(']',start);
        if(end==std::string::npos) throw std::runtime_error("not an isolation array");
        line=line.substr(start,end-start);
        for(auto &c:line) if(c==','||c=='('||c==')') c=' ';
        std::istringstream values(line); std::vector<double> result; double v;
        while(values>>v) {
            if(!std::isfinite(v)) throw std::runtime_error("nonfinite number");
            result.push_back(v);
        }
        if(!values.eof()) throw std::runtime_error("invalid number");
        return result;
    }
    throw std::runtime_error("missing array");
}
std::vector<Point> points(std::string const &path) {
    auto values=array(path,"points"); if(values.size()%3) throw std::runtime_error("point width");
    std::vector<Point> result(values.size()/3);
    for(size_t i=0;i<values.size();++i) {
        result[i/3].p[i%3]=float(values[i]);
        if(!std::isfinite(result[i/3].p[i%3])) throw std::runtime_error("point overflow");
    }
    return result;
}
int main(int argc,char **argv) try {
    if(argc<3) throw std::runtime_error("usage: compare_osd cage.usda level1.usda [vertex ...] [--normal-probe NEW_DIRECTORY]");
    auto src=points(argv[1]), expected=points(argv[2]);
    auto c=array(argv[1],"faceVertexCounts"), idx=array(argv[1],"faceVertexIndices");
    if(src.empty() || c.empty()) throw std::runtime_error("empty cage");
    for(double n:c) if(n!=3) throw std::runtime_error("only triangle cages supported");
    if(idx.size()!=c.size()*3) throw std::runtime_error("corner count mismatch");
    for(double i:idx) if(i<0 || i>=src.size() || std::floor(i)!=i) throw std::runtime_error("invalid index");
    std::vector<int> counts(c.begin(),c.end()), indices(idx.begin(),idx.end());
    for(size_t i=0;i<indices.size();i+=3)
        if(indices[i]==indices[i+1] || indices[i]==indices[i+2] || indices[i+1]==indices[i+2])
            throw std::runtime_error("repeated face vertex");
    Far::TopologyDescriptor desc; desc.numVertices=src.size(); desc.numFaces=counts.size();
    desc.numVertsPerFace=counts.data(); desc.vertIndicesPerFace=indices.data();
    Sdc::Options scheme; scheme.SetVtxBoundaryInterpolation(Sdc::Options::VTX_BOUNDARY_EDGE_AND_CORNER);
    using Factory=Far::TopologyRefinerFactory<Far::TopologyDescriptor>;
    std::unique_ptr<Far::TopologyRefiner> ref(Factory::Create(desc,Factory::Options(Sdc::SCHEME_CATMARK,scheme)));
    if(!ref) throw std::runtime_error("invalid topology");
    Far::TopologyRefiner::UniformOptions uniform(1); uniform.fullTopologyInLastLevel=true;
    ref->RefineUniform(uniform);
    auto const &base=ref->GetLevel(0);
    std::vector<Point> native(ref->GetLevel(1).GetNumVertices());
    Far::PrimvarRefinerReal<double>(*ref).Interpolate(1,src,native);
    std::vector<Point> limit(native.size()), du(native.size()), dv(native.size());
    Far::PrimvarRefinerReal<double>(*ref).Limit(native,limit,du,dv);
    if(native.size()!=expected.size()) throw std::runtime_error("refined point count mismatch");
    std::vector<int> map;
    for(int i=0;i<base.GetNumVertices();++i) map.push_back(base.GetVertexChildVertex(i));
    for(int i=0;i<base.GetNumFaces();++i) map.push_back(base.GetFaceChildVertex(i));
    std::map<std::pair<int,int>,int> edges;
    for(int i=0;i<base.GetNumEdges();++i) {
        auto v=base.GetEdgeVertices(i); edges[std::minmax(v[0],v[1])]=base.GetEdgeChildVertex(i);
    }
    for(auto const &[edge,child]:edges) map.push_back(child);
    if(map.size()!=expected.size() || std::set<int>(map.begin(),map.end()).size()!=map.size())
        throw std::runtime_error("invalid child mapping");
    double maximum=0; size_t worst=0, failures=0;
    for(size_t i=0;i<map.size();++i) {
        double error=0; for(int k=0;k<3;++k) error=std::max(error,std::abs(native.at(map[i]).p[k]-expected[i].p[k]));
        for(double value:native.at(map[i]).p) if(!std::isfinite(value)) throw std::runtime_error("nonfinite refinement");
        if(error>maximum) { maximum=error; worst=i; }
        failures+=error>1e-7;
    }
    std::cout<<"points="<<native.size()<<" max_component_error="<<maximum<<" worst_vertex="<<worst<<" over_1e-7="<<failures<<'\n';
    for(int arg=3;arg<argc;++arg) {
        if(std::string(argv[arg])=="--normal-probe") {
            if(arg+2!=argc || failures) throw std::runtime_error("normal probe requires matching positions and final NEW_DIRECTORY");
            std::ostringstream normals; normals<<std::setprecision(9)<<"    normal3f[] normals = [";
            std::ostringstream positions; positions<<std::setprecision(9)<<"    point3f[] points = [";
            std::set<size_t> referenced;
            for(double index:array(argv[2],"faceVertexIndices")) {
                if(index<0 || index>=map.size() || std::floor(index)!=index) throw std::runtime_error("invalid refined index");
                referenced.insert(size_t(index));
            }
            size_t zero=0, zero_referenced=0;
            for(size_t i=0;i<map.size();++i) {
                auto const &a=du.at(map[i]), &b=dv.at(map[i]);
                double n[3]={a.p[1]*b.p[2]-a.p[2]*b.p[1],a.p[2]*b.p[0]-a.p[0]*b.p[2],a.p[0]*b.p[1]-a.p[1]*b.p[0]};
                double length=std::hypot(n[0],n[1],n[2]);
                if(!std::isfinite(length)) throw std::runtime_error("nonfinite limit normal");
                zero+=length==0;
                zero_referenced+=length==0 && referenced.count(i);
                for(auto &v:n) v=length==0?0:v/length;
                normals<<(i?",(":"(")<<n[0]<<','<<n[1]<<','<<n[2]<<')';
                auto const &p=limit.at(map[i]);
                for(double v:p.p) if(!std::isfinite(v)) throw std::runtime_error("nonfinite limit position");
                positions<<(i?",(":"(")<<p.p[0]<<','<<p.p[1]<<','<<p.p[2]<<')';
            }
            normals<<']'; positions<<']';
            std::ifstream input(argv[2]); std::string line; std::ostringstream output, surface; int replaced=0, replaced_points=0;
            while(std::getline(input,line)) {
                if(line.find("normal3f[] normals = [")!=std::string::npos) { line=normals.str(); ++replaced; }
                output<<line<<'\n';
                if(line.find("point3f[] points = [")!=std::string::npos) { line=positions.str(); ++replaced_points; }
                surface<<line<<'\n';
            }
            if(replaced!=1 || replaced_points!=1 || input.bad()) throw std::runtime_error("missing or duplicate probe array");
            std::filesystem::path dir=argv[++arg];
            if(!std::filesystem::create_directory(dir)) throw std::runtime_error("output directory already exists");
            std::ofstream file(dir/"with_limit_normals.usda"); file<<output.str(); file.close();
            if(!file) throw std::runtime_error("normal probe write failed");
            std::ofstream surface_file(dir/"with_limit_surface.usda"); surface_file<<surface.str(); surface_file.close();
            if(!surface_file) throw std::runtime_error("surface probe write failed");
            std::cout<<"limit_normal_probe="<<dir<<" zero_normals="<<zero<<" zero_referenced_normals="<<zero_referenced<<'\n';
            break;
        }
        std::string value=argv[arg]; size_t parsed=0; int i=std::stoi(value,&parsed);
        if(parsed!=value.size() || i<0 || i>=int(map.size())) throw std::runtime_error("invalid witness index");
        auto const &p=native[map[i]];
        std::cout.precision(12); std::cout<<"vertex="<<i<<" native="<<p.p[0]<<','<<p.p[1]<<','<<p.p[2]<<'\n';
        auto const &a=du[map[i]], &b=dv[map[i]];
        double n[3]={a.p[1]*b.p[2]-a.p[2]*b.p[1],a.p[2]*b.p[0]-a.p[0]*b.p[2],a.p[0]*b.p[1]-a.p[1]*b.p[0]};
        std::cout<<"  limit_tangent_cross="<<n[0]<<','<<n[1]<<','<<n[2]<<'\n';
    }
    return failures?1:0;
} catch(std::exception const &e) { std::cerr<<e.what()<<'\n'; return 2; }
